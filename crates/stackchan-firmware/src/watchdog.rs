//! Embassy task watchdog: per-channel heartbeat counters that surface
//! silent task deaths.
//!
//! Each periodic producer task (audio RMS loop, IMU, ambient, power,
//! head) calls [`Channel::beat`] once per loop iteration. Every
//! `WATCHDOG_PERIOD_MS`, the watchdog task reads each counter, diffs
//! against the previous poll, and emits a `defmt::warn!` if the actual
//! delta falls below the channel's expected minimum.
//!
//! Why heartbeats instead of peeking the existing `Signal<…, T>`
//! channels: `Signal` is single-consumer (each `try_take` is destructive),
//! so a watchdog drain would race the render task that's already
//! consuming the same signal. Heartbeats sidestep the race entirely.
//!
//! The cost is one `AtomicU32::fetch_add(1, Relaxed)` per producer
//! iteration — a few cycles, no embassy task wake, no contention with
//! the existing Signal semantics.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Ticker};

/// How often the watchdog task wakes to check every channel. 5 s gives
/// even the slowest producer (1 Hz power task) a comfortable margin —
/// 5 expected beats per window — while still flagging a wedged task
/// within roughly 5 s of the symptom appearing.
const WATCHDOG_PERIOD_MS: u64 = 5_000;

/// One monitored producer task.
///
/// Cadence-aware: `min_per_window` is the smallest beat count we accept
/// inside a `WATCHDOG_PERIOD_MS` poll window before we consider the
/// channel silent. Set conservatively (~50% of nominal) to absorb
/// scheduler jitter without false-positives.
pub struct Channel {
    /// Human-readable name surfaced in the warning log.
    name: &'static str,
    /// Total beats since boot. Producer increments via [`Channel::beat`].
    counter: AtomicU32,
    /// Counter value at the previous watchdog poll. The watchdog task
    /// swaps this in `check_and_reset`.
    last_polled: AtomicU32,
    /// Minimum beats expected per `WATCHDOG_PERIOD_MS` window.
    min_per_window: u32,
}

impl Channel {
    /// Construct a heartbeat channel for a producer task.
    const fn new(name: &'static str, min_per_window: u32) -> Self {
        Self {
            name,
            counter: AtomicU32::new(0),
            last_polled: AtomicU32::new(0),
            min_per_window,
        }
    }

    /// Producer-side: increment the heartbeat counter. Call once per
    /// producer-task loop iteration. `Relaxed` because the watchdog
    /// only needs eventual visibility, not strict ordering.
    pub fn beat(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Watchdog-side: read the current counter, swap it into
    /// `last_polled`, and return the delta + whether it's below the
    /// `min_per_window` threshold.
    fn check_and_reset(&self) -> (u32, bool) {
        let now = self.counter.load(Ordering::Relaxed);
        let prev = self.last_polled.swap(now, Ordering::Relaxed);
        let delta = now.wrapping_sub(prev);
        let stale = delta < self.min_per_window;
        (delta, stale)
    }
}

// ---- Channel statics --------------------------------------------------------
//
// `min_per_window` values are derived from each producer's nominal
// cadence × WATCHDOG_PERIOD_MS, halved to absorb jitter:
//   audio  RMS @ ~33 ms  → ~151 nominal → min 75
//   imu    @ 10 ms       → 500 nominal  → min 300
//   ambient @ 500 ms     → 10 nominal   → min 5
//   power  @ 1000 ms     → 5 nominal    → min 3
//   head   @ 20 ms       → 250 nominal  → min 150

/// Audio RX-RMS loop. Beats once per published `AudioRms` sample
/// (~one per 33 ms window).
pub static AUDIO: Channel = Channel::new("audio", 75);
/// BMI270 IMU polling loop. Beats once per 10 ms iteration.
pub static IMU: Channel = Channel::new("imu", 300);
/// LTR-553 ambient-light polling loop. Beats once per 500 ms iteration.
pub static AMBIENT: Channel = Channel::new("ambient", 5);
/// AXP2101 battery + USB-power polling loop. Beats once per 1 s iteration.
pub static POWER: Channel = Channel::new("power", 3);
/// `SCServo` head-update loop. Beats once per 20 ms command tick.
pub static HEAD: Channel = Channel::new("head", 150);

/// One row of [`TasksSnapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskHealth {
    /// Channel name. Stable identifier the dashboard renders.
    pub name: &'static str,
    /// Beats observed in the last `WATCHDOG_PERIOD_MS` window.
    pub delta: u32,
    /// Minimum acceptable beats per window — below this is stale.
    pub min_per_window: u32,
    /// Whether the last poll's delta fell below `min_per_window`.
    pub stale: bool,
}

/// Read-side mirror of every monitored channel. Updated by
/// [`run_watchdog_loop`] after each `check_all`; HTTP reads via
/// [`read_tasks_snapshot`].
#[derive(Debug, Clone, Copy)]
pub struct TasksSnapshot {
    /// Per-channel health, in declaration order.
    pub channels: [TaskHealth; 5],
    /// Window length in ms — surfaced so the dashboard can label the
    /// "beats in last X" without hard-coding it on the JS side.
    pub window_ms: u64,
}

impl TasksSnapshot {
    /// Construct a snapshot with zero-delta entries for every channel.
    /// Used as the initial value of [`TASKS_SNAPSHOT`] before the
    /// watchdog has run its first poll.
    const fn empty() -> Self {
        Self {
            channels: [
                TaskHealth {
                    name: "audio",
                    delta: 0,
                    min_per_window: AUDIO_MIN_PER_WINDOW,
                    stale: false,
                },
                TaskHealth {
                    name: "imu",
                    delta: 0,
                    min_per_window: IMU_MIN_PER_WINDOW,
                    stale: false,
                },
                TaskHealth {
                    name: "ambient",
                    delta: 0,
                    min_per_window: AMBIENT_MIN_PER_WINDOW,
                    stale: false,
                },
                TaskHealth {
                    name: "power",
                    delta: 0,
                    min_per_window: POWER_MIN_PER_WINDOW,
                    stale: false,
                },
                TaskHealth {
                    name: "head",
                    delta: 0,
                    min_per_window: HEAD_MIN_PER_WINDOW,
                    stale: false,
                },
            ],
            window_ms: WATCHDOG_PERIOD_MS,
        }
    }
}

/// Audio RMS loop — 33 ms tick, 50% jitter margin.
const AUDIO_MIN_PER_WINDOW: u32 = 75;
/// IMU polling loop — 10 ms tick, 50% jitter margin.
const IMU_MIN_PER_WINDOW: u32 = 300;
/// Ambient-light polling loop — 500 ms tick, 50% jitter margin.
const AMBIENT_MIN_PER_WINDOW: u32 = 5;
/// AXP2101 polling loop — 1 s tick, 50% jitter margin.
const POWER_MIN_PER_WINDOW: u32 = 3;
/// `SCServo` head loop — 20 ms tick, 50% jitter margin.
const HEAD_MIN_PER_WINDOW: u32 = 150;

/// Read-side mirror of every channel's last-poll outcome. Updated
/// by [`run_watchdog_loop`] after each `check_all`; HTTP reads via
/// [`read_tasks_snapshot`].
static TASKS_SNAPSHOT: Mutex<CriticalSectionRawMutex, core::cell::Cell<TasksSnapshot>> =
    Mutex::new(core::cell::Cell::new(TasksSnapshot::empty()));

/// Cheap snapshot read for the HTTP `GET /tasks` handler.
#[must_use]
pub fn read_tasks_snapshot() -> TasksSnapshot {
    TASKS_SNAPSHOT.lock(core::cell::Cell::get)
}

/// Iterate every channel, warn on staleness, and refresh
/// [`TASKS_SNAPSHOT`]. Internal helper, but exposed for unit
/// testability.
fn check_all() {
    let channels: &[&Channel] = &[&AUDIO, &IMU, &AMBIENT, &POWER, &HEAD];
    // Pull `name` + `min_per_window` from the source `Channel` struct
    // each tick rather than the hardcoded `empty()` table — if the
    // `channels` slice ever reorders or grows out of sync with the
    // `empty()` initialiser, the displayed names would silently
    // disagree with reality.
    let mut snap = TasksSnapshot::empty();
    for (i, ch) in channels.iter().enumerate() {
        let (delta, stale) = ch.check_and_reset();
        if stale {
            defmt::warn!(
                "watchdog: channel '{=str}' silent — saw {=u32} beats in {=u64} ms (expected ≥ {=u32})",
                ch.name,
                delta,
                WATCHDOG_PERIOD_MS,
                ch.min_per_window,
            );
        }
        snap.channels[i] = TaskHealth {
            name: ch.name,
            delta,
            min_per_window: ch.min_per_window,
            stale,
        };
    }
    TASKS_SNAPSHOT.lock(|cell| cell.set(snap));
}

/// Embassy task entry point. Spawned once from `main.rs`. Runs the
/// supervisor poll forever; never panics, never returns.
pub async fn run_watchdog_loop() -> ! {
    let mut ticker = Ticker::every(Duration::from_millis(WATCHDOG_PERIOD_MS));
    defmt::info!(
        "watchdog task: {=u64} ms tick, monitoring 5 channels (audio, imu, ambient, power, head)",
        WATCHDOG_PERIOD_MS,
    );
    // Discard the first window — boot ordering means several producers
    // haven't beaten yet at this point, and a false-positive flood on
    // boot trains the operator to ignore the warnings.
    ticker.next().await;
    let _ = (
        AUDIO.check_and_reset(),
        IMU.check_and_reset(),
        AMBIENT.check_and_reset(),
        POWER.check_and_reset(),
        HEAD.check_and_reset(),
    );
    loop {
        ticker.next().await;
        check_all();
    }
}
