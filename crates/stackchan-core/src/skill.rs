//! [`Skill`] — longer-running NPC capability with discoverable
//! metadata.
//!
//! Each skill self-describes when to fire (its
//! [`SkillMeta::description`] is meant for human readers and a future
//! dispatcher) and what to do once fired. The [`crate::Director`]
//! polls each registered skill's [`Skill::should_fire`] predicate per
//! frame and invokes those that return `true`.
//!
//! Skills don't write `entity.face` or `entity.motor` directly. They
//! express intent through `entity.mind`, `entity.voice`, and
//! `entity.events`; modifiers in [`crate::director::Phase::Expression`]
//! and [`crate::director::Phase::Motion`] translate that intent into
//! rendered face and physical motion. The rule is documented; a
//! `SkillView<'a>` borrow type that enforces it via the type system is
//! sketched but not implemented.
//!
//! ## Lifecycle
//!
//! 1. [`Skill::meta`] — static identity, description, priority.
//! 2. [`Skill::should_fire`] — polled each frame. Multiple skills may
//!    fire on the same frame; order is by [`SkillMeta::priority`]
//!    (higher first).
//! 3. [`Skill::invoke`] — the action. Returns [`SkillStatus::Done`]
//!    (finished) or [`SkillStatus::Continuing`] (re-invoke next frame
//!    while `should_fire` stays `true`).
//!
//! Persistent cross-frame state lives inside the skill; cleanup on
//! deactivation is detected by tracking the should-fire→false
//! transition internally.

use crate::director::SkillMeta;
use crate::entity::Entity;

/// Status returned by [`Skill::invoke`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatus {
    /// One-shot complete, or finished a continuing run. The Director
    /// won't invoke this skill again until [`Skill::should_fire`]
    /// transitions back to `true` from `false`.
    Done,
    /// The skill wants to be invoked again next frame as long as
    /// `should_fire` continues to return `true`.
    Continuing,
}

/// A discoverable NPC capability. See the module docs for the
/// face/motor write prohibition.
pub trait Skill {
    /// Identity, description, priority.
    fn meta(&self) -> &'static SkillMeta;

    /// Trigger predicate. Polled each frame; `true` causes
    /// [`Skill::invoke`] to be called.
    fn should_fire(&self, entity: &Entity) -> bool;

    /// Invoke the skill. Returns [`SkillStatus::Done`] if finished or
    /// [`SkillStatus::Continuing`] if it wants to be invoked again
    /// next frame (subject to `should_fire`).
    fn invoke(&mut self, entity: &mut Entity) -> SkillStatus;
}
