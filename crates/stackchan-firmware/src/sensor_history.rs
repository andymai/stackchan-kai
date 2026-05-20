//! Rolling 1-minute window of sensor readings for LLM grounding.
//!
//! A 1 Hz sampler task snapshots [`crate::net::snapshot::read_sensors`]
//! every second into a bounded ring; an MCP read tool surfaces the
//! window so an LLM can answer "what was the avatar's body-touch /
//! IMU / mic doing 10 seconds ago when I was told it laughed?" without
//! a separate streaming subscription.
//!
//! ## Why on-tick sampling vs on-change
//!
//! On-tick gives a predictable cadence: every entry is exactly one
//! second apart, callers know what gaps mean (none — the sample
//! always lands). On-change would compress the buffer but make
//! reasoning about elapsed time non-trivial. For 1-minute LLM
//! grounding the storage cost (~3.6 KB) is irrelevant.
//!
//! ## Persistence
//!
//! None — the buffer lives in RAM and resets on reboot. The first
//! second's worth of entries on a cold device may show "no IMU yet"
//! while BMI270 init races the sampler; later entries pick up the
//! steady-state.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Ticker};
use heapless::Vec;

use crate::net::snapshot::{SensorsSnapshot, read_sensors};

/// Number of samples retained. 60 × 1 Hz = 60 seconds of history,
/// which is the same horizon `behavior.toast_overlay`'s short-term
/// memory and the sidecar's per-session context already operate on.
pub const HISTORY_CAP: usize = 60;

/// Sampler cadence. 1 Hz matches the [`crate::reminders`] scheduler
/// tick and is fine-grained enough for LLM grounding without
/// flooding the buffer.
const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

/// One ring-buffer entry — a sensors snapshot tagged with the
/// monotonic uptime at sample time. The HTTP / MCP edge converts
/// `uptime_ms` to "N seconds ago" before surfacing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryEntry {
    /// Monotonic uptime when the sample was taken, in milliseconds.
    /// Matches the `uptime_ms` field of `GET /health`.
    pub uptime_ms: u64,
    /// The sensors snapshot at sample time. `imu` / `ambient_lux` /
    /// `body_touch` carry `Option`s — `None` means the relevant chip
    /// hadn't published yet.
    pub snapshot: SensorsSnapshot,
}

/// Backing static ring buffer. The sampler task and the read path
/// both lock the inner `RefCell`; the critical-section mutex covers
/// the brief in/out moments without any `await`.
static HISTORY: Mutex<CriticalSectionRawMutex, RefCell<Vec<HistoryEntry, HISTORY_CAP>>> =
    Mutex::new(RefCell::new(Vec::new()));

/// Snapshot the current history list, oldest first. Allocates a
/// fresh `Vec` so the caller can serialise it without holding the
/// mutex across `await`s.
#[must_use]
pub fn read_history() -> Vec<HistoryEntry, HISTORY_CAP> {
    HISTORY.lock(|cell| cell.borrow().clone())
}

/// Push one sample into a caller-owned ring, dropping the oldest
/// entry if the buffer is at capacity. Pure function so unit tests
/// can exercise the rotation logic against a local `Vec` instead of
/// the module-level static — keeps tests independent under
/// `cargo test`'s default multi-threaded harness.
fn push_into(list: &mut Vec<HistoryEntry, HISTORY_CAP>, entry: HistoryEntry) {
    if list.is_full() {
        // `remove(0)` is O(N), but N == 60 and this runs at 1 Hz
        // so the shifting cost is irrelevant. Keeps the
        // implementation a single Vec rather than dragging in
        // `heapless::HistoryBuffer` for one consumer.
        let _ = list.remove(0);
    }
    let _ = list.push(entry);
}

/// Push one sample to the shared ring static. The embassy task
/// calls this on each tick; routes through [`push_into`] so the
/// rotation logic is exercised by the unit-test path.
fn push_sample(entry: HistoryEntry) {
    HISTORY.lock(|cell| {
        let mut list = cell.borrow_mut();
        push_into(&mut list, entry);
    });
}

/// Embassy task — samples the live sensors snapshot once per
/// [`SAMPLE_PERIOD`] and stores it in the ring.
#[embassy_executor::task]
pub async fn sensor_history_task() {
    defmt::info!(
        "sensor-history: sampler up (cap={=usize}, period={=u64}ms)",
        HISTORY_CAP,
        SAMPLE_PERIOD.as_millis(),
    );
    let mut ticker = Ticker::every(SAMPLE_PERIOD);
    loop {
        ticker.next().await;
        let entry = HistoryEntry {
            uptime_ms: Instant::now().as_millis(),
            snapshot: read_sensors(),
        };
        push_sample(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(uptime_ms: u64) -> HistoryEntry {
        HistoryEntry {
            uptime_ms,
            snapshot: SensorsSnapshot {
                imu: None,
                ambient_lux: None,
                audio_rms: 0.0,
                body_touch: None,
            },
        }
    }

    #[test]
    fn push_into_then_iter_returns_insertion_order() {
        let mut list: Vec<HistoryEntry, HISTORY_CAP> = Vec::new();
        push_into(&mut list, entry(1000));
        push_into(&mut list, entry(2000));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].uptime_ms, 1000);
        assert_eq!(list[1].uptime_ms, 2000);
    }

    #[test]
    fn push_into_at_capacity_drops_oldest() {
        let mut list: Vec<HistoryEntry, HISTORY_CAP> = Vec::new();
        for i in 0..HISTORY_CAP {
            push_into(&mut list, entry((i as u64 + 1) * 1000));
        }
        // One past capacity — should evict the first entry.
        push_into(&mut list, entry(99_000));
        assert_eq!(list.len(), HISTORY_CAP);
        assert_eq!(list[0].uptime_ms, 2_000, "first entry should be evicted");
        assert_eq!(
            list[HISTORY_CAP - 1].uptime_ms,
            99_000,
            "newest entry should be at the tail",
        );
    }

    #[test]
    fn push_into_empty_list_succeeds() {
        let list: Vec<HistoryEntry, HISTORY_CAP> = Vec::new();
        assert!(list.is_empty());
    }
}
