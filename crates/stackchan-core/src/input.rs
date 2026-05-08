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

use alloc::sync::Arc;

use crate::dance::DanceScript;
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
    /// Set [`crate::Attention::Point`] toward an explicit 3D world
    /// point and hold for `hold_ms`. Right-handed coordinates with
    /// `+Z` forward, `+X` right, `+Y` up; only the direction matters
    /// (target distance is irrelevant). The modifier graph converts
    /// the point to a head pose via [`crate::Pose::from_xyz_lookat`];
    /// targets at the origin (singularity) are rejected by the
    /// firmware-side parser before this variant ever lands.
    LookAtPoint {
        /// Cartesian world point `(x, y, z)`.
        target: (f32, f32, f32),
        /// Hold duration in milliseconds.
        hold_ms: u32,
    },
    /// Clear any active emotion or look-at hold and return to
    /// autonomous behavior.
    Reset,
    /// Open a listen window — set [`crate::Intent::Listening`] +
    /// [`crate::Attention::Listening`] for `duration_ms`, queue an
    /// acknowledge chirp, and arm the [`crate::Decorator::Ear`]
    /// overlay. Sourced from the dashboard `POST /listen`, the
    /// long-press body-touch trigger, or a future wake-word event.
    /// Fire-and-forget from the producer's view; the modifier owns
    /// the timeout.
    StartListen {
        /// How long to hold the listening state, in milliseconds.
        /// Operator-driven calls typically use 3 000 ms.
        duration_ms: u32,
    },
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
    /// Open an ESP-NOW pairing window for `duration_ms` milliseconds.
    /// While the window is active,
    /// [`crate::modifiers::RemoteCommandModifier`] arms the
    /// [`crate::Decorator::Pairing`] overlay (refreshing expiry each
    /// tick) so the operator gets visible confirmation, and the
    /// firmware-side ESP-NOW receiver accepts new peer registrations.
    /// Fire-and-forget on autonomy — pairing does not gate emotion or
    /// attention.
    EnterPairing {
        /// Window length in milliseconds. The decorator refreshes its
        /// 500 ms tail each tick and fades on release.
        duration_ms: u32,
    },
}

/// Pending inputs the modifier graph consumes.
///
/// Persistent across frames: the [`Director`](crate::Director) does
/// not clear `Input`. Modifiers consume explicitly by setting fields
/// back to their default.
///
/// Only `PartialEq` (not `Eq`) because [`RemoteCommand`] carries a
/// [`Pose`] with `f32` fields. Not `Copy` because `dance_script`
/// holds an `Arc` — the `Arc` clone is intentional on the consume
/// path.
#[derive(Debug, Default, Clone, PartialEq)]
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
    /// Pending dance script uploaded via `POST /dance` and waiting
    /// for [`crate::modifiers::DancePlayer`] to consume it. Cleared
    /// (set to `None`) by the player on the tick it loads the
    /// script. The `Arc` makes the firmware → modifier handoff
    /// O(1) regardless of keyframe count.
    pub dance_script: Option<Arc<DanceScript>>,
}
