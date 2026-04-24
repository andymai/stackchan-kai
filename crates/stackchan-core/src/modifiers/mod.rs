//! Animation modifiers.
//!
//! A [`Modifier`] is a stateful object that mutates an [`Avatar`] in response
//! to the passage of time. Modifiers are driven by a render loop (firmware)
//! or a simulated clock (sim crate); they never allocate and never panic.
//!
//! The canonical stack, outermost-first, is:
//!
//! 1. [`EmotionCycle`] (or application code) — sets `Avatar::emotion`.
//! 2. [`EmotionStyle`] — translates emotion into style fields, with a
//!    linear ease over the transition window.
//! 3. [`Blink`] — drives eye open/closed phase, reading `open_weight` and
//!    `blink_rate_scale` from the avatar.
//! 4. [`Breath`] — vertical drift on all features, scaled by
//!    `breath_depth_scale`.
//! 5. [`IdleDrift`] — occasional eye-center jitter.
//! 6. [`IdleSway`] — slow pan/tilt head wander written to
//!    `Avatar::head_pose`. Non-visual; drives the firmware's head-update
//!    task, not the pixel pipeline.
//!
//! [`Avatar`]: crate::avatar::Avatar

mod blink;
mod breath;
mod emotion_cycle;
mod emotion_style;
mod idle_drift;
mod idle_sway;

pub use blink::Blink;
pub use breath::Breath;
pub use emotion_cycle::EmotionCycle;
pub use emotion_style::EmotionStyle;
pub use idle_drift::IdleDrift;
pub use idle_sway::IdleSway;

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
