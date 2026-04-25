//! One-frame fire flags propagating modifier-detected edges to the
//! firmware's render loop.
//!
//! [`Events`] is a flat struct of `bool` fields, each representing an
//! edge a modifier detected this frame. The firmware reads these after
//! `App::run()` returns, between the modifier pass and the post-render
//! work (e.g. enqueuing audio chirps that pair with `Voice::chirp_request`).
//!
//! ## Lifecycle
//!
//! [`crate::app::App::run`] **clears** `entity.events` at the start of
//! each frame, before any modifier runs. Modifiers set fields to `true`
//! when their state machine fires an edge. The firmware reads them
//! after the modifier pass; on the next frame, App clears them again.
//!
//! No modifier should ever read `entity.events.*` from the *current*
//! frame — they're firmware-facing signals, not inter-modifier
//! communication. (Inter-modifier coordination flows through
//! [`crate::mind::Affect`] / [`crate::mind::Autonomy`] / [`crate::voice::Voice`].)
//!
//! ## Why a struct of bools, not a queue
//!
//! Each event is idempotent within a frame: at most one pickup edge
//! per frame, at most one wake edge, etc. A queue would add allocation
//! and ordering questions for no benefit. The `bool`-per-edge shape
//! also makes the firmware-side dispatch trivially branchless.

/// One-frame fire flags. Cleared by [`crate::Director::run`] at frame
/// start; set by modifiers; read by firmware between modifier pass
/// and post-render work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Events {
    /// `PickupReaction` detected a pickup edge this frame.
    pub pickup_fired: bool,
    /// `WakeOnVoice` detected a sustained-voice edge this frame.
    pub wake_fired: bool,
    /// Camera-mode toggle entered. Currently set by the firmware's
    /// camera-mode handler, not a modifier; reserved here for
    /// completeness so all firmware-facing edges live in one place.
    pub camera_mode_entered: bool,
    /// Camera-mode toggle exited.
    pub camera_mode_exited: bool,
    /// `LowBatteryEmotion` armed (downward-edge crossing of the enter
    /// threshold while unplugged). Used by the firmware to fire the
    /// low-battery alert chirp once per crossing.
    pub low_battery_armed: bool,
}
