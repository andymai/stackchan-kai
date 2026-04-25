//! The [`Skill`] trait — discoverable NPC capabilities.
//!
//! **Status:** trait surface only. No implementations today.
//!
//! Skills are modeled on the **Claude Code Skill** pattern: each skill
//! self-describes *when to fire* (the [`SkillMeta::description`] field
//! doubles as trigger guidance for human readers and, in v2.x,
//! LAN-host-LLM dispatch) and *what to do once fired* (the trait body).
//! This makes skills a literal capability menu: today the [`crate::Director`]
//! polls each skill's [`Skill::should_fire`] predicate; tomorrow a
//! cognition bridge can read all skills' descriptions and pick which
//! to invoke.
//!
//! ## The single most important rule
//!
//! **Skills MUST NOT write to `entity.face` or `entity.motor` directly.**
//! Skills express intent through `entity.mind`, `entity.voice`, and
//! `entity.events`; modifiers in [`crate::director::Phase::Expression`]
//! and [`crate::director::Phase::Motion`] translate that intent into
//! rendered face and physical motion.
//!
//! This invariant is doc-enforced today (Rust doesn't prevent a
//! misbehaving skill from reaching into `face`). v2.x will introduce a
//! `SkillView<'a>` borrow type that mechanically excludes `face` /
//! `motor` from the writable surface, turning the rule into a compile
//! error. Until then, treating it as a hard rule is the architectural
//! contract.
//!
//! ## Lifecycle
//!
//! Three methods total:
//!
//! 1. [`Skill::meta`] — static identity + description + arbitration data.
//! 2. [`Skill::should_fire`] — predicate the [`crate::Director`] polls
//!    every frame. Returning `true` causes the skill to be invoked.
//!    Multiple skills can fire on the same frame; ordering is by
//!    [`SkillMeta::priority`] (higher first).
//! 3. [`Skill::invoke`] — the action. Returns [`SkillStatus::Done`]
//!    (one-shot finished) or [`SkillStatus::Continuing`] (poll
//!    `should_fire` again next frame).
//!
//! Skills that need persistent cross-frame state hold it internally
//! (e.g. a `last_fired: Option<Instant>` field). Skills that need
//! cleanup-on-deactivation can detect the should-fire→false transition
//! by tracking that internally too. v2.x may grow an `on_exit` hook
//! with a default no-op impl if the pattern needs it.

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

/// A discoverable NPC capability.
///
/// See module-level docs for the **face/motor write prohibition**.
pub trait Skill {
    /// Static identity + description + arbitration metadata.
    fn meta(&self) -> &'static SkillMeta;

    /// Trigger predicate. Polled by [`crate::Director`] each frame;
    /// `true` causes [`Skill::invoke`] to be called.
    fn should_fire(&self, entity: &Entity) -> bool;

    /// Invoke the skill. Returns [`SkillStatus::Done`] if the skill
    /// has finished or [`SkillStatus::Continuing`] if it wants to be
    /// invoked again next frame (subject to `should_fire`).
    fn invoke(&mut self, entity: &mut Entity) -> SkillStatus;
}
