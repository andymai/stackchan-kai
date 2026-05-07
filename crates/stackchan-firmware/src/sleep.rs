//! Operator-commanded sleep mode.
//!
//! Distinct from [`stackchan_core::Dormancy`] — the latter is the
//! autonomous idle state that flips on activity-timeout and quiets a
//! handful of motion modifiers. Sleep mode is a stronger,
//! operator-driven hand-off: eyes shut, head limp, LED ring dark,
//! audio TX queue paused. Wake is any touch, body-touch, or explicit
//! HTTP / MCP `wake` command.
//!
//! ## Why firmware-side, not in `stackchan-core`
//!
//! Sleep is observable through hardware-task gating (head task skips
//! `set_pose`, LED task drops the ring to off, render task overrides
//! the face geometry to closed-eyes) — every consumer is on the
//! firmware side. Pulling this into the `Mind` modifier graph would
//! force the simulator + every host-test rig to know about a
//! firmware-only concern. A small firmware static + a face-state
//! override at render time keeps the surface contained.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

/// Sleep state — operator-controlled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SleepState {
    /// Default. Render, head, LED, audio TX all run normally.
    #[default]
    Awake,
    /// Operator commanded sleep. The render task forces eyes shut +
    /// mouth flat, the head task skips `set_pose`, the LED task
    /// drops the ring to off, the audio TX queue is paused.
    Sleeping,
}

impl SleepState {
    /// Lowercase wire string for `/state` JSON + the HTTP route bodies.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Awake => "awake",
            Self::Sleeping => "sleeping",
        }
    }
}

/// Latest operator-commanded sleep state.
///
/// Producers (HTTP `POST /sleep` / `/wake`, MCP `sleep` / `wake`,
/// touch / body-touch tasks on wake) signal this; the render task
/// drains it into [`STATE_CACHE`] each tick. Latest-wins semantics.
pub static SLEEP_SIGNAL: Signal<CriticalSectionRawMutex, SleepState> = Signal::new();

/// Cached current sleep state — single source of truth for HTTP
/// readback and for the per-task gates that don't sit on the render
/// loop. Updated by the render task each frame from
/// [`SLEEP_SIGNAL`].
pub static STATE_CACHE: Mutex<CriticalSectionRawMutex, Cell<SleepState>> =
    Mutex::new(Cell::new(SleepState::Awake));

/// Snapshot the current sleep state.
#[must_use]
pub fn current() -> SleepState {
    STATE_CACHE.lock(Cell::get)
}

/// True iff the operator has commanded sleep.
#[must_use]
pub fn is_sleeping() -> bool {
    matches!(current(), SleepState::Sleeping)
}

/// Drain the signal into the cache. Called once per render tick by
/// the render task; safe to call from any task that wants to mirror
/// the latest signal value into [`STATE_CACHE`].
pub fn pump() {
    if let Some(next) = SLEEP_SIGNAL.try_take() {
        STATE_CACHE.lock(|cell| cell.set(next));
        defmt::info!("sleep: state → {=str}", next.wire_str());
    }
}

/// Producer-side helper for the wake-on-touch path.
///
/// Pushes [`SleepState::Awake`] onto the signal if the cache
/// currently reports `Sleeping`. No-op when already awake — avoids
/// burning the signal slot for a no-op transition.
pub fn wake_if_sleeping() {
    if is_sleeping() {
        SLEEP_SIGNAL.signal(SleepState::Awake);
    }
}

/// Apply the sleep override to the rendered face.
///
/// Called by the render task after `Director::run` and before
/// `Face::draw` so the modifier pipeline runs normally while sleep
/// just clamps the outputs. Keeping the override at the render edge
/// means waking up resumes the live modifier state without a
/// transition glitch.
pub fn apply_to_face(face: &mut stackchan_core::Face) {
    if !is_sleeping() {
        return;
    }
    face.left_eye.phase = stackchan_core::EyePhase::Closed;
    face.left_eye.weight = 0;
    face.right_eye.phase = stackchan_core::EyePhase::Closed;
    face.right_eye.weight = 0;
    face.mouth.weight = 0;
    face.mouth.mouth_open = 0.0;
    face.style.eye_curve = 0;
    face.style.mouth_curve = 0;
    face.style.cheek_blush = 0;
    // Clear symbolic overlays — a heart decorator on a sleeping
    // avatar reads as confused.
    face.decorator = None;
    face.bubble = None;
}
