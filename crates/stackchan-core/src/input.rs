//! Pending firmware → modifier inputs.
//!
//! [`Input`] carries requests from firmware tasks that the modifier
//! graph consumes. Unlike [`crate::events::Events`] (one-frame fire
//! flags cleared by the [`Director`](crate::Director) at frame start),
//! `Input` survives across frames until a modifier consumes it.
//!
//! Producer side: a firmware task drains a Signal channel and writes
//! the relevant `Input` field (e.g. the touch task writes
//! `entity.input.tap_pending = true`).
//!
//! Consumer side: the modifier checks the field each tick, and if set,
//! reads + clears it.

/// Pending inputs the modifier graph consumes.
///
/// Persistent across frames: the [`Director`](crate::Director) does
/// not clear `Input`. Modifiers consume explicitly by setting fields
/// back to their default.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    /// Tap edge from the touch sensor or power button. Consumed by
    /// [`crate::modifiers::EmotionFromTouch`].
    pub tap_pending: bool,
    /// Most recent decoded IR-remote `(address, command)` pair.
    /// Consumed by [`crate::modifiers::EmotionFromRemote`].
    pub remote_pending: Option<(u16, u8)>,
}
