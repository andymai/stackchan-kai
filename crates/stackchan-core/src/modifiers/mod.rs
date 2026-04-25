//! Animation modifiers.
//!
//! A [`Modifier`] is a stateful object that mutates an [`Avatar`] in response
//! to the passage of time. Modifiers are driven by a render loop (firmware)
//! or a simulated clock (sim crate); they never allocate and never panic.
//!
//! The canonical stack, outermost-first, is:
//!
//! 1. [`EmotionTouch`] — consumes queued taps, advances
//!    `Avatar::emotion`, and writes `Avatar::manual_until`. Runs first
//!    so the same tick that produced the tap also clears any expired
//!    override before `EmotionCycle` checks it.
//! 2. [`RemoteCommand`] — consumes IR-remote `(address, command)`
//!    pairs from the firmware RMT task, looks them up in a user-
//!    supplied mapping table, and writes emotion + `manual_until`.
//!    Runs after `EmotionTouch` so a just-cleared hold is visible;
//!    stands down if any other modifier already set one.
//! 3. [`PickupReaction`] — reads `Avatar::accel_g`, flips emotion to
//!    `Surprised` with a `manual_until` hold when a pickup / drop is
//!    detected. Stands down when `manual_until` is already set (so
//!    explicit touch / remote wins).
//! 4. [`AmbientSleepy`] — reads `Avatar::ambient_lux`, flips emotion
//!    to `Sleepy` with a short `manual_until` hold in dark rooms
//!    (hysteresis 20/50 lux). Runs after `PickupReaction` so a
//!    pickup-in-the-dark still surfaces as Surprised rather than
//!    Sleepy.
//! 5. [`LowBatteryEmotion`] — reads `Avatar::battery_percent`, forces
//!    `Sleepy` (with a short `manual_until` hold) when the `SoC` drops
//!    below a threshold. Runs alongside `AmbientSleepy` as the
//!    "environmental override" group; touch / pickup / remote still
//!    win since this respects an existing `manual_until` hold.
//! 6. [`EmotionCycle`] (or application code) — sets `Avatar::emotion`
//!    when `manual_until` is unset or expired.
//! 7. [`EmotionStyle`] — translates emotion into style fields, with a
//!    linear ease over the transition window.
//! 8. [`Blink`] — drives eye open/closed phase, reading `open_weight` and
//!    `blink_rate_scale` from the avatar.
//! 9. [`Breath`] — vertical drift on all features, scaled by
//!    `breath_depth_scale`.
//! 10. [`IdleDrift`] — occasional eye-center jitter.
//! 11. [`IdleSway`] — slow pan/tilt head wander written to
//!     `Avatar::head_pose`. Non-visual; drives the firmware's head-update
//!     task, not the pixel pipeline.
//! 12. [`EmotionHead`] — emotion-keyed pan/tilt bias added on top of the
//!     sway. Runs **after** `IdleSway` so bias composes additively rather
//!     than fighting for absolute control of the pose.
//! 13. [`MouthOpenAudio`] — reads `Avatar::mouth.weight` preserved by
//!     earlier modifiers, writes `Avatar::mouth.mouth_open` from
//!     microphone RMS via a dB-mapped attack/release envelope. Runs
//!     last in the visual stack so emotion geometry stays the static
//!     "at rest" shape and audio drives the dynamic open-amount on top.
//!
//! [`Avatar`]: crate::avatar::Avatar

mod ambient_sleepy;
mod blink;
mod breath;
mod emotion_cycle;
mod emotion_head;
mod emotion_style;
mod emotion_touch;
mod idle_drift;
mod idle_sway;
mod low_battery;
mod mouth_open_audio;
mod pickup_reaction;
mod remote_command;

pub use ambient_sleepy::{AMBIENT_HOLD_MS, AmbientSleepy, SLEEPY_ENTER_LUX, SLEEPY_EXIT_LUX};
pub use blink::Blink;
pub use breath::Breath;
pub use emotion_cycle::EmotionCycle;
pub use emotion_head::EmotionHead;
pub use emotion_style::EmotionStyle;
pub use emotion_touch::{EMOTION_ORDER, EmotionTouch, MANUAL_HOLD_MS};
pub use idle_drift::IdleDrift;
pub use idle_sway::IdleSway;
#[allow(
    deprecated,
    reason = "re-exporting deprecated alias for downstream compat"
)]
pub use low_battery::LOW_BATTERY_THRESHOLD_PERCENT;
pub use low_battery::{
    LOW_BATTERY_ENTER_PERCENT, LOW_BATTERY_EXIT_PERCENT, LOW_BATTERY_HOLD_MS, LowBatteryEmotion,
};
pub use mouth_open_audio::{
    DEFAULT_ATTACK_MS, DEFAULT_FULL_DB, DEFAULT_RELEASE_MS, DEFAULT_SILENCE_DB, MouthOpenAudio,
};
pub use pickup_reaction::{PICKUP_DEBOUNCE_MS, PICKUP_DEVIATION_G, PickupReaction};
pub use remote_command::{RemoteCommand, RemoteMapping};

use crate::avatar::Avatar;
use crate::clock::Instant;

/// Trait implemented by every animation modifier.
///
/// `update` is called once per tick with the current wall time. The modifier
/// mutates the avatar directly based on its internal state machine; the
/// render loop re-reads the avatar after every call.
pub trait Modifier {
    /// Advance the modifier's internal state to `now` and apply its effect
    /// to `avatar`.
    fn update(&mut self, avatar: &mut Avatar, now: Instant);
}
