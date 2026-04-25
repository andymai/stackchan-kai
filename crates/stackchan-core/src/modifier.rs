//! The [`Modifier`] trait — per-frame state mutators.
//!
//! Modifiers are the primary unit of behavior in the v0.x animation
//! model: stateful objects whose `update` method is called once per
//! frame to evolve the entity's state. They are deliberately *narrow*:
//!
//! - Per-frame, not lifecycle-driven (use [`crate::Skill`] for that).
//! - Synchronous, no I/O, no allocation.
//! - Mutate `Entity` directly; no return value.
//!
//! Modifiers register with a [`crate::Director`] which sorts them by
//! [`crate::director::Phase`] and `priority`, then iterates each frame.
//! The phase enum encodes the canonical NPC tick order
//! (Perception → Cognition → Affect → Speech → Expression → Motion →
//! Audio → Output); registration order within a phase + priority
//! determines fine ordering.
//!
//! ## Implementing
//!
//! ```ignore
//! use stackchan_core::{Modifier, ModifierMeta, Phase, Field, Entity};
//!
//! struct Twitch { state: u8 }
//!
//! impl Modifier for Twitch {
//!     fn meta(&self) -> &'static ModifierMeta {
//!         static META: ModifierMeta = ModifierMeta {
//!             name: "Twitch",
//!             description: "Adds occasional eye-twitches when emotion is Surprised.",
//!             phase: Phase::Expression,
//!             priority: 0,
//!             reads: &[Field::Emotion, Field::EyeScale],
//!             writes: &[Field::LeftEyeWeight, Field::RightEyeWeight],
//!         };
//!         &META
//!     }
//!     fn update(&mut self, entity: &mut Entity) {
//!         let now = entity.tick.now;
//!         // ...
//!     }
//! }
//! ```

use crate::director::ModifierMeta;
use crate::entity::Entity;

/// A per-frame state mutator on the entity.
pub trait Modifier {
    /// Static metadata for this modifier type.
    fn meta(&self) -> &'static ModifierMeta;

    /// Advance the modifier's state to `entity.tick.now` and apply its
    /// effect to `entity`.
    fn update(&mut self, entity: &mut Entity);
}
