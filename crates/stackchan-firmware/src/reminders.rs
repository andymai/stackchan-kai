//! In-memory reminder/timer scheduler.
//!
//! Holds a small fixed-capacity list of pending entries. Each entry
//! pairs a fire deadline (monotonic [`Instant`]) with a
//! [`ScheduledAction`] — either a baked phrase to play or a named
//! motion to run. A 1 Hz embassy task drains due entries and
//! dispatches each through the same control-plane signals that HTTP,
//! MCP, and ESP-NOW use, so the audio + motion paths stay uniform.
//!
//! ## Why monotonic, not wall-clock
//!
//! Wall-clock-based reminders need an SNTP-synced RTC, calendar
//! arithmetic, and a story for what happens when the clock is
//! corrected mid-flight. Monotonic deadlines (boot-relative) sidestep
//! all of that for the v0.2.0 MVP — operators schedule "in N seconds"
//! reminders, not absolute calendar times. A wall-clock variant can
//! ship as a follow-up that reuses this scheduler with a converter on
//! the input side.
//!
//! ## Persistence
//!
//! None — the list lives in RAM and resets on reboot. Reminders are
//! short-horizon by definition (5-day cap); the operator-tooling
//! cost of re-arming after a reboot is acceptable.

use alloc::sync::Arc;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Ticker};
use heapless::Vec;
use stackchan_core::RemoteCommand;
use stackchan_core::motion::NamedMotion;
use stackchan_core::voice::{Locale, PhraseId, Priority};

use crate::net::http::{DANCE_SCRIPT_SIGNAL, enqueue_remote_command};

/// Maximum simultaneous reminders. 16 is well beyond plausible
/// operator use (a desk-toy isn't a calendar) and bounded enough to
/// keep the static buffer small.
pub const MAX_REMINDERS: usize = 16;

/// Cap on the seconds-from-now operators can schedule.
///
/// Five days is generous for any legitimate desk-toy reminder; the
/// cap exists to surface accidental "fire in 31536000 seconds"
/// inputs as a validation error rather than a silent five-year-
/// from-now sleep.
pub const MAX_REMINDER_HORIZON_SECS: u64 = 5 * 24 * 60 * 60;

/// Tick cadence for the dispatcher task. 1 Hz — operator-facing
/// reminders don't need sub-second precision and a slower poll keeps
/// the firmware's idle power floor low.
const REMINDER_TICK: Duration = Duration::from_secs(1);

/// Monotonic id counter for `create_reminder` returns. Wraps at
/// `u32::MAX` (a single device would have to schedule 4 billion
/// reminders to wrap), but the wrap is OK — no caller relies on
/// `id` ordering.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// Shared reminder list. The dispatcher task and the HTTP/MCP create
/// paths both touch this static; the critical-section mutex covers
/// the brief in/out moments without any await.
static REMINDERS: Mutex<CriticalSectionRawMutex, RefCell<Vec<Reminder, MAX_REMINDERS>>> =
    Mutex::new(RefCell::new(Vec::new()));

/// What happens when a [`Reminder`] fires.
///
/// Adding a new variant requires updating
/// [`render_reminders_json`][crate::net::http] so the operator-facing
/// JSON surfaces a recognisable discriminator field, plus the
/// dispatcher's match arm in [`reminders_task`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScheduledAction {
    /// Play a baked phrase through the speech path on fire.
    Speak(PhraseId),
    /// Play a canonical one-shot motion (greet / nod / shake / laugh)
    /// through the dance-player path on fire.
    PlayMotion(NamedMotion),
}

/// One scheduled reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    /// Stable identifier returned by `create_reminder` /
    /// `schedule_motion` and consumed by `cancel_reminder`.
    pub id: u32,
    /// Monotonic deadline at which the reminder fires.
    pub deadline: Instant,
    /// What to do when the deadline passes.
    pub action: ScheduledAction,
}

/// Operator-supplied creation request. Validated into a [`Reminder`]
/// by [`add_reminder`] before insertion.
#[derive(Debug, Clone, Copy)]
pub struct CreateRequest {
    /// Seconds from `now` until the reminder should fire. Must be
    /// `> 0` and `<= MAX_REMINDER_HORIZON_SECS`.
    pub fire_in_secs: u64,
    /// What the dispatcher does when the reminder fires.
    pub action: ScheduledAction,
}

/// Validation errors surfaced to the HTTP / MCP edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderError {
    /// `fire_in_secs == 0`. Use `speak` for fire-now playback.
    NotInTheFuture,
    /// `fire_in_secs` exceeded [`MAX_REMINDER_HORIZON_SECS`].
    HorizonExceeded,
    /// The reminder list is full.
    QueueFull,
}

/// Insert a reminder. Returns the assigned id, or a typed error.
///
/// # Errors
///
/// - [`ReminderError::NotInTheFuture`] if `fire_in_secs == 0`.
/// - [`ReminderError::HorizonExceeded`] if `fire_in_secs` is past the
///   advertised cap.
/// - [`ReminderError::QueueFull`] if the static buffer is full.
pub fn add_reminder(now: Instant, req: CreateRequest) -> Result<u32, ReminderError> {
    if req.fire_in_secs == 0 {
        return Err(ReminderError::NotInTheFuture);
    }
    if req.fire_in_secs > MAX_REMINDER_HORIZON_SECS {
        return Err(ReminderError::HorizonExceeded);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let reminder = Reminder {
        id,
        deadline: now + Duration::from_secs(req.fire_in_secs),
        action: req.action,
    };
    REMINDERS.lock(|cell| {
        let mut list = cell.borrow_mut();
        list.push(reminder).map_err(|_| ReminderError::QueueFull)
    })?;
    Ok(id)
}

/// Remove the reminder with `id`. Returns `true` if a matching entry
/// was found and dropped, `false` otherwise (caller surfaces a 404).
pub fn cancel_reminder(id: u32) -> bool {
    REMINDERS.lock(|cell| {
        let mut list = cell.borrow_mut();
        list.iter().position(|r| r.id == id).is_some_and(|idx| {
            list.swap_remove(idx);
            true
        })
    })
}

/// Snapshot the current reminder list. Allocates a fresh `Vec` so
/// the caller can hold it across awaits without keeping the mutex.
#[must_use]
pub fn list_reminders() -> Vec<Reminder, MAX_REMINDERS> {
    REMINDERS.lock(|cell| cell.borrow().clone())
}

/// Drain every reminder whose deadline is `<= now`, returning them
/// in firing order. Used by both the dispatcher task and tests.
fn drain_due(now: Instant) -> Vec<Reminder, MAX_REMINDERS> {
    REMINDERS.lock(|cell| {
        let mut list = cell.borrow_mut();
        let mut due: Vec<Reminder, MAX_REMINDERS> = Vec::new();
        let mut i = 0;
        while i < list.len() {
            if list[i].deadline <= now {
                let r = list.swap_remove(i);
                let _ = due.push(r);
            } else {
                i += 1;
            }
        }
        due
    })
}

/// Embassy task — drains due reminders at `REMINDER_TICK` cadence
/// and dispatches each through its action-appropriate channel.
/// `Speak` rides the same `REMOTE_COMMAND_QUEUE` that HTTP / MCP /
/// ESP-NOW speak paths use; `PlayMotion` signals the dance player
/// directly. The queue absorbs short bursts, so simultaneously-due
/// reminders are all preserved; the per-tick spacing is conservative
/// pacing for the consumers downstream, not a correctness requirement
/// of the control-plane signals themselves.
#[embassy_executor::task]
pub async fn reminders_task() {
    defmt::info!(
        "reminders: dispatcher up (cap={=usize}, horizon={=u64}s)",
        MAX_REMINDERS,
        MAX_REMINDER_HORIZON_SECS,
    );
    let mut ticker = Ticker::every(REMINDER_TICK);
    loop {
        ticker.next().await;
        let due = drain_due(Instant::now());
        for r in due {
            match r.action {
                ScheduledAction::Speak(phrase) => {
                    defmt::info!(
                        "reminders: firing id={=u32} speak={:?}",
                        r.id,
                        defmt::Debug2Format(&phrase),
                    );
                    enqueue_remote_command(RemoteCommand::Speak {
                        phrase,
                        locale: Locale::En,
                        priority: Priority::Normal,
                    });
                }
                ScheduledAction::PlayMotion(motion) => {
                    defmt::info!(
                        "reminders: firing id={=u32} motion={:?}",
                        r.id,
                        defmt::Debug2Format(&motion),
                    );
                    DANCE_SCRIPT_SIGNAL.signal(Arc::new(motion.script()));
                }
            }
            // The render task drains REMOTE_COMMAND_QUEUE once per
            // ~33 ms render frame; signalling a second reminder
            // before that drain runs would silently overwrite the
            // first (Signal is single-waker, latest-wins). Pace
            // bursts so each signalled command is consumed before
            // the next replaces it. The same spacing applies to
            // motion fires that hit DANCE_SCRIPT_SIGNAL (also a
            // single-waker signal).
            embassy_time::Timer::after(embassy_time::Duration::from_millis(
                REMINDER_BURST_SPACING_MS,
            ))
            .await;
        }
    }
}

/// Spacing between back-to-back reminder fires when several land
/// due in the same tick. Sized comfortably above the 33 ms render
/// cadence so each consumer drains its signal between pushes:
/// `Speak` actions push to `REMOTE_COMMAND_QUEUE` (drained by the
/// render task per frame), `PlayMotion` actions push to
/// `DANCE_SCRIPT_SIGNAL` (drained by the same render-task tick that
/// also runs the `DancePlayer` modifier). Both signals are
/// single-waker / latest-wins, so pacing matters for both.
const REMINDER_BURST_SPACING_MS: u64 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_state() {
        REMINDERS.lock(|cell| cell.borrow_mut().clear());
        NEXT_ID.store(1, Ordering::SeqCst);
    }

    #[test]
    fn add_then_drain_returns_due_only() {
        reset_state();
        let t0 = Instant::from_ticks(0);
        let id1 = add_reminder(
            t0,
            CreateRequest {
                fire_in_secs: 1,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        )
        .unwrap();
        let _id2 = add_reminder(
            t0,
            CreateRequest {
                fire_in_secs: 60,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        )
        .unwrap();

        let due = drain_due(t0 + Duration::from_secs(2));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id1);
        // The far-future reminder remains.
        assert_eq!(list_reminders().len(), 1);
    }

    #[test]
    fn add_rejects_zero_horizon() {
        reset_state();
        let err = add_reminder(
            Instant::from_ticks(0),
            CreateRequest {
                fire_in_secs: 0,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        );
        assert_eq!(err, Err(ReminderError::NotInTheFuture));
    }

    #[test]
    fn add_rejects_horizon_above_cap() {
        reset_state();
        let err = add_reminder(
            Instant::from_ticks(0),
            CreateRequest {
                fire_in_secs: MAX_REMINDER_HORIZON_SECS + 1,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        );
        assert_eq!(err, Err(ReminderError::HorizonExceeded));
    }

    #[test]
    fn cancel_returns_false_for_unknown_id() {
        reset_state();
        assert!(!cancel_reminder(9999));
    }

    #[test]
    fn cancel_removes_matching_id() {
        reset_state();
        let id = add_reminder(
            Instant::from_ticks(0),
            CreateRequest {
                fire_in_secs: 30,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        )
        .unwrap();
        assert!(cancel_reminder(id));
        assert!(list_reminders().is_empty());
        // Second cancel of the same id is a no-op.
        assert!(!cancel_reminder(id));
    }

    #[test]
    fn add_rejects_when_full() {
        reset_state();
        for _ in 0..MAX_REMINDERS {
            add_reminder(
                Instant::from_ticks(0),
                CreateRequest {
                    fire_in_secs: 30,
                    action: ScheduledAction::Speak(PhraseId::WakeChirp),
                },
            )
            .unwrap();
        }
        let err = add_reminder(
            Instant::from_ticks(0),
            CreateRequest {
                fire_in_secs: 30,
                action: ScheduledAction::Speak(PhraseId::WakeChirp),
            },
        );
        assert_eq!(err, Err(ReminderError::QueueFull));
    }
}
