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

use crate::emotion::Emotion;
use crate::head::Pose;
use crate::voice::{Locale, PhraseId, Priority};

/// External command delivered through the firmware control plane.
///
/// Producer: the firmware HTTP task parses a request body into one of
/// these variants and writes [`Input::remote_command`].
///
/// Consumer: [`crate::modifiers::RemoteCommandModifier`] drains the
/// slot, stashes any hold timer internally, and re-asserts emotion or
/// attention each frame until the timer expires.
///
/// Only `PartialEq` (not `Eq`) because [`RemoteCommand::LookAt`]
/// carries a [`Pose`] with `f32` fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RemoteCommand {
    /// Set [`crate::Affect::emotion`] and hold the autonomy gate for
    /// `hold_ms` so autonomous emotion drivers stand down. Source is
    /// recorded as [`crate::OverrideSource::Remote`].
    SetEmotion {
        /// Emotion to assert.
        emotion: Emotion,
        /// Hold duration in milliseconds. Zero is fire-and-forget:
        /// emotion is asserted once and autonomy is released on the
        /// same tick.
        hold_ms: u32,
    },
    /// Set [`crate::Attention::Tracking`] toward `target` and hold for
    /// `hold_ms` so the tracking modifier does not stomp the target.
    LookAt {
        /// Head pose to look at, in the same coordinate system as
        /// `motor.head_pose`.
        target: Pose,
        /// Hold duration in milliseconds.
        hold_ms: u32,
    },
    /// Clear any active emotion or look-at hold and return to
    /// autonomous behavior.
    Reset,
    /// Play a [`PhraseId`] from the baked TTS catalog through the
    /// firmware's TX path. Fire-and-forget — no avatar-state hold,
    /// no autonomy gate. The firmware drains this slot before
    /// `Director::run` and dispatches via the audio queue;
    /// [`crate::modifiers::RemoteCommandModifier`] sees this variant
    /// only as a defensive no-op.
    Speak {
        /// Catalog entry to render (chirp, beep, or verbal phrase).
        phrase: PhraseId,
        /// Locale for verbal phrases. Ignored for non-verbal chirps.
        locale: Locale,
        /// Queue priority. Higher priorities preempt currently-
        /// playing audio; the default is [`Priority::Normal`].
        priority: Priority,
    },
}

/// Pending inputs the modifier graph consumes.
///
/// Persistent across frames: the [`Director`](crate::Director) does
/// not clear `Input`. Modifiers consume explicitly by setting fields
/// back to their default.
///
/// Only `PartialEq` (not `Eq`) because [`RemoteCommand`] carries a
/// [`Pose`] with `f32` fields.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Input {
    /// Tap edge from the touch sensor or power button. Consumed by
    /// [`crate::modifiers::EmotionFromTouch`].
    pub tap_pending: bool,
    /// Most recent decoded IR-remote `(address, command)` pair.
    /// Consumed by [`crate::modifiers::EmotionFromRemote`].
    pub remote_pending: Option<(u16, u8)>,
    /// Most recent external control-plane command. Consumed by
    /// [`crate::modifiers::RemoteCommandModifier`].
    pub remote_command: Option<RemoteCommand>,
}
