//! Operator-facing permission decision flow.
//!
//! Subscribes to [`crate::ble::buddy::BUDDY_INBOUND`] for snapshot
//! prompts and to the body-touch sensor for back-of-head taps, and
//! sends [`Outbound::Permission`] back on
//! [`crate::ble::buddy::BUDDY_OUTBOUND`].
//!
//! ## Sleep + chime
//!
//! A `prompt` going from absent → present wakes the LCD (in case the
//! avatar had drifted off) and queues a [`PhraseId::PickupChirp`] —
//! a brief upward sweep designed to draw attention without being
//! intrusive. The autonomy gate is left to
//! [`crate::buddy_render`]; this task only cares about the decision.
//!
//! ## Tap-twice gesture
//!
//! Single taps are too easy to fat-finger on a device that lives on
//! a desk surrounded by other things. The pattern instead requires
//! two taps of the same zone within
//! [`TAP_TWICE_WINDOW_MS`] for a decision to commit. The left zone
//! decides **deny**, the right zone decides **approve once** (the
//! only two values the wire protocol carries today). The center
//! zone clears any armed half-tap.
//!
//! Once a decision goes out, the prompt id is latched so a stale
//! body-touch read can't ack the same prompt twice.
//!
//! Touch detection runs on release edges (`prior > 0 → current ==
//! 0`) rather than press edges so picking the device up doesn't
//! commit a held contact.

use embassy_futures::select::{Either, select};
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Instant, Ticker};
use stackchan_buddy_proto::{Decision, Inbound, Outbound, Snapshot};
use stackchan_core::voice::{Locale, PhraseId, Priority};
use stackchan_core::{Clock, RemoteCommand};

use crate::ble::buddy::{BUDDY_INBOUND, BUDDY_OUTBOUND};
use crate::clock::HalClock;
use crate::net::http::REMOTE_COMMAND_SIGNAL;
use crate::net::snapshot;
use crate::sleep;
use crate::toast::{self, ToastLevel};

// Gesture and watchdog timestamps ride `embassy_time::Instant`
// because the polling loop ticks on it and gesture math wants
// `Duration` arithmetic. Toast pushes ride `HalClock.now()` because
// `crate::toast` is shared with the core engine and types its
// timestamps as `stackchan_core::Instant`. The two clocks are
// independent; nothing in this module compares across them.

/// Body-touch poll cadence. Matches [`crate::body_touch`]'s sensor
/// poll so a fresh release edge surfaces within one tick of when it
/// actually happened.
const POLL_PERIOD_MS: u64 = 50;

/// How long a single armed tap stays valid before the gesture
/// resets. Three seconds is long enough for a deliberate two-tap
/// from someone walking up to the device, short enough that an
/// accidental brush + later real tap doesn't combine into a
/// commit.
const TAP_TWICE_WINDOW_MS: u64 = 3_000;

/// How long to suppress duplicate decision sends for the same
/// prompt id. The desktop withdraws the prompt within one snapshot
/// of receiving our reply (typically <1 s); 5 s is comfortable
/// belt-and-suspenders against a late re-send.
const DECISION_LATCH_MS: u64 = 5_000;

/// Which zone the operator armed for the next tap to commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmedZone {
    /// Awaiting a second left-zone tap to commit a deny.
    Left,
    /// Awaiting a second right-zone tap to commit an approve.
    Right,
}

/// Per-zone press / release edge detector. Holds the prior poll's
/// intensity so the next poll can recognise a release.
#[derive(Debug, Clone, Copy, Default)]
struct ZoneEdge {
    /// `true` while the zone has been continuously touched since the
    /// last release edge. Reset by `tick`.
    held: bool,
}

impl ZoneEdge {
    /// Update from a fresh intensity sample. Returns `true` on the
    /// release edge (was held, now not).
    const fn tick(&mut self, intensity: u8) -> bool {
        let pressed = intensity > 0;
        let released = self.held && !pressed;
        self.held = pressed;
        released
    }
}

/// Permission task. Spawns once at boot, runs for the firmware
/// lifetime.
#[embassy_executor::task]
pub async fn buddy_permission_task() -> ! {
    let Ok(mut sub) = BUDDY_INBOUND.subscriber() else {
        defmt::error!(
            "buddy_permission: BUDDY_INBOUND subscriber slot exhausted; task parking forever"
        );
        loop {
            embassy_time::Timer::after(Duration::from_secs(3600)).await;
        }
    };
    defmt::info!("buddy_permission: armed (tap-twice on left=deny / right=approve)");

    let mut state = State::default();
    let mut ticker = Ticker::every(Duration::from_millis(POLL_PERIOD_MS));

    loop {
        match select(sub.next_message(), ticker.next()).await {
            Either::First(WaitResult::Message(m)) => on_message(m, &mut state),
            Either::First(WaitResult::Lagged(n)) => {
                defmt::warn!(
                    "buddy_permission: subscriber lagged, dropped {=u64} message(s)",
                    n
                );
            }
            Either::Second(()) => on_tick(&mut state),
        }
    }
}

/// All the state the task tracks across polls + messages.
#[derive(Debug, Default)]
struct State {
    /// Id of the prompt currently displayed, if any. Cleared when
    /// the desktop withdraws the prompt OR after we commit a
    /// decision.
    active_prompt_id: Option<alloc::string::String>,
    /// Wall-clock arrival time of the active prompt. Used to
    /// (later) decorate the operator-facing UX with elapsed
    /// seconds.
    prompt_arrived_at: Option<Instant>,
    /// Half-tap awaiting confirmation, if any.
    armed: Option<(ArmedZone, Instant)>,
    /// Last id we sent a decision for + when. Suppresses duplicate
    /// sends if a late body-touch edge fires before the desktop
    /// withdraws the prompt.
    last_decided: Option<(alloc::string::String, Instant)>,
    /// Press / release edge detector for the left zone (deny).
    left_edge: ZoneEdge,
    /// Press / release edge detector for the center zone (cancel).
    centre_edge: ZoneEdge,
    /// Press / release edge detector for the right zone (approve).
    right_edge: ZoneEdge,
}

/// React to one inbound buddy message.
fn on_message(message: Inbound, state: &mut State) {
    if let Inbound::Snapshot(snap) = message {
        on_snapshot(&snap, state);
    }
}

/// Latch a fresh prompt and fire the operator notifications (LCD
/// wake + chime + toast). Used both when no prompt was active
/// (rising edge from idle) and when a different prompt id arrives
/// in place of an existing one — replacement is operator-visible
/// too, so it needs the same side effects.
fn notify_new_prompt(
    state: &mut State,
    new_id: alloc::string::String,
    replaced: Option<alloc::string::String>,
) {
    if let Some(prev) = replaced {
        defmt::info!(
            "buddy_permission: prompt replaced ({=str} → {=str})",
            prev.as_str(),
            new_id.as_str()
        );
    } else {
        defmt::info!(
            "buddy_permission: prompt arrived (id={=str})",
            new_id.as_str()
        );
    }
    state.active_prompt_id = Some(new_id);
    state.prompt_arrived_at = Some(Instant::now());
    state.armed = None;
    sleep::wake_if_sleeping();
    queue_phrase(PhraseId::PickupChirp);
    toast::push(ToastLevel::Warn, "approve? tap-twice back", HalClock.now());
}

/// React to one heartbeat snapshot. Handles both the rising edge
/// (new prompt appeared) and the falling edge (prompt withdrawn).
fn on_snapshot(snap: &Snapshot, state: &mut State) {
    // `alloc::string::String` rather than a fixed-cap `heapless`
    // string so an unusually long id (the spec doesn't bound it)
    // can't collapse to `""` and conflict with another oversized id
    // in the decision latch.
    let inbound_id = snap
        .prompt
        .as_ref()
        .map(|p| alloc::string::String::from(p.id.as_str()));

    let active_id = state.active_prompt_id.clone();

    match (active_id, inbound_id) {
        (None, Some(new_id)) => notify_new_prompt(state, new_id, None),
        (Some(prev), Some(new)) if prev != new => notify_new_prompt(state, new, Some(prev)),
        (Some(_), None) => {
            // Falling edge — desktop withdrew the prompt before we
            // committed a decision (user clicked Approve in the
            // desktop UI, or the request timed out upstream).
            defmt::info!("buddy_permission: prompt withdrawn upstream");
            state.active_prompt_id = None;
            state.prompt_arrived_at = None;
            state.armed = None;
        }
        _ => {}
    }
}

/// One body-touch poll. Drives gesture state forward and may emit a
/// decision.
fn on_tick(state: &mut State) {
    expire_stale(state);

    let Some(touch) = snapshot::read_sensors().body_touch else {
        // Sensor not yet initialised; nothing to do.
        return;
    };

    let left_released = state.left_edge.tick(touch.left);
    let centre_released = state.centre_edge.tick(touch.centre);
    let right_released = state.right_edge.tick(touch.right);

    if !left_released && !centre_released && !right_released {
        return;
    }

    // Only act on edges while a prompt is actually pending —
    // bystander touches must not arm anything.
    if state.active_prompt_id.is_none() {
        return;
    }

    if centre_released && state.armed.is_some() {
        defmt::info!("buddy_permission: armed gesture cancelled (centre)");
        state.armed = None;
        return;
    }

    if left_released {
        handle_zone_tap(state, ArmedZone::Left);
    } else if right_released {
        handle_zone_tap(state, ArmedZone::Right);
    }
}

/// Apply the tap-twice state machine for one zone tap.
fn handle_zone_tap(state: &mut State, zone: ArmedZone) {
    let now = Instant::now();
    match state.armed {
        Some((armed_zone, _)) if armed_zone == zone => {
            commit(state, zone, now);
        }
        _ => {
            defmt::info!(
                "buddy_permission: armed ({=str}); tap again to confirm",
                match zone {
                    ArmedZone::Left => "deny",
                    ArmedZone::Right => "approve",
                }
            );
            state.armed = Some((zone, now));
        }
    }
}

/// Commit a decision: send the outbound reply, queue a confirm
/// chirp, and latch the id so a late repeat tap is ignored.
fn commit(state: &mut State, zone: ArmedZone, now: Instant) {
    let Some(prompt_id) = state.active_prompt_id.clone() else {
        return;
    };
    if let Some((last_id, when)) = &state.last_decided
        && last_id == &prompt_id
        && now.duration_since(*when) < Duration::from_millis(DECISION_LATCH_MS)
    {
        // Already sent for this prompt; suppress the duplicate.
        return;
    }
    let decision = match zone {
        ArmedZone::Left => Decision::Deny,
        ArmedZone::Right => Decision::Once,
    };
    let chirp = match zone {
        ArmedZone::Left => PhraseId::StartleChirp,
        ArmedZone::Right => PhraseId::WakeChirp,
    };
    defmt::info!(
        "buddy_permission: decision {=str} → {=str}",
        prompt_id.as_str(),
        decision.as_wire(),
    );
    BUDDY_OUTBOUND.signal(Outbound::Permission {
        id: prompt_id.as_str().into(),
        decision,
    });
    queue_phrase(chirp);
    state.last_decided = Some((prompt_id, now));
    state.active_prompt_id = None;
    state.prompt_arrived_at = None;
    state.armed = None;
}

/// Drop an armed half-tap older than the window. Also evicts the
/// last-decided latch entry once it has aged past the suppression
/// window.
fn expire_stale(state: &mut State) {
    let now = Instant::now();
    if let Some((_, when)) = state.armed
        && now.duration_since(when) >= Duration::from_millis(TAP_TWICE_WINDOW_MS)
    {
        defmt::trace!("buddy_permission: armed gesture timed out");
        state.armed = None;
    }
    if let Some((_, when)) = &state.last_decided
        && now.duration_since(*when) >= Duration::from_millis(DECISION_LATCH_MS)
    {
        state.last_decided = None;
    }
}

/// Enqueue a non-verbal SFX through the same path the rest of the
/// firmware uses for confirmation tones.
fn queue_phrase(phrase: PhraseId) {
    REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::Speak {
        phrase,
        locale: Locale::En,
        priority: Priority::Normal,
    });
}
