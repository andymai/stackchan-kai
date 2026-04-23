//! Animation modifiers.
//!
//! A [`Modifier`] is a stateful object that mutates an [`Avatar`] in response
//! to the passage of time. Modifiers are driven by a render loop (firmware)
//! or a simulated clock (sim crate); they never allocate and never panic.
//!
//! [`Avatar`]: crate::avatar::Avatar

mod blink;
mod breath;
mod idle_drift;

pub use blink::Blink;
pub use breath::Breath;
pub use idle_drift::IdleDrift;

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
