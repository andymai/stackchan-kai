//! Pending inputs that modifiers consume on the frame they're set.
//!
//! [`Input`] is the firmware's "what just happened" channel into the
//! modifier graph. Unlike [`crate::events::Events`] (one-frame fire
//! flags that the [`Director`](crate::Director) clears at frame start),
//! `Input` carries pending requests that survive across frames until
//! a modifier explicitly consumes them.
//!
//! ## Producer / consumer pattern
//!
//! - **Producer** (firmware tasks): drain a Signal channel, set the
//!   relevant `Input` field. e.g. the touch task drains `TAP_SIGNAL`
//!   and writes `entity.input.tap_pending = true`.
//! - **Consumer** (modifier `update`): on each tick, check the field.
//!   If set, take + clear it (e.g. `tap_pending = false`,
//!   `remote_pending = None`) and apply the effect.
//!
//! This replaces the v0.x imperative-method pattern (`emotion_touch.tap()`,
//! `remote_command.queue(addr, cmd)`, `mouth_open_audio.set_rms(rms)`)
//! which couldn't coexist with the [`Director`](crate::Director)
//! borrowing each modifier mutably for the duration of the registry.
//! Inputs flow through `Entity` instead — the same channel as everything
//! else.

/// Pending inputs the modifier graph consumes.
///
/// Persistent across frames: the [`Director`](crate::Director) does NOT
/// clear `Input` at frame start. Modifiers explicitly consume by
/// setting fields back to their default (`false` / `None`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    /// Tap edge from a touch sensor or button. Consumed by
    /// [`crate::modifiers::EmotionTouch`]: it advances emotion + sets
    /// the autonomy hold, then clears this flag.
    pub tap_pending: bool,
    /// Most recent decoded IR-remote `(address, command)` pair.
    /// Consumed by [`crate::modifiers::RemoteCommand`] which looks the
    /// pair up in its mapping table, sets emotion, and clears this.
    pub remote_pending: Option<(u16, u8)>,
}
