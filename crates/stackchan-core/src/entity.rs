//! Top-level entity model: the NPC.
//!
//! [`Entity`] composes the entity's six concerns into named
//! sub-components: [`Face`] (visual), [`Motor`] (motion), [`Perception`]
//! (sensors), [`Voice`] (speech I/O), [`Mind`] (brain), [`Events`]
//! (one-frame fire flags). A seventh field, [`Tick`], is bookkeeping
//! that [`crate::Director`] stamps each frame.
//!
//! ## Why "Entity" and not "Avatar"
//!
//! `Avatar` (v0.x) implied "visual representation" — a face animator.
//! StackChan's roadmap is an AI-powered NPC: conversation, memory,
//! intent, dialogue. The visual face is one component of the entity,
//! not the whole thing. The rename + decomposition keeps the type
//! system aligned with the domain ontology.
//!
//! ## Modifier vs Skill access patterns
//!
//! - **Modifiers** ([`crate::Modifier`]) take `&mut Entity` and can
//!   touch any sub-component, but conventionally each modifier writes
//!   only the components matching its [`crate::director::Phase`].
//! - **Skills** ([`crate::Skill`]) take `&mut Entity` but **must
//!   not** write to `face` or `motor` directly — they write to
//!   `mind` / `voice` / `events`, and modifiers in
//!   [`crate::director::Phase::Expression`] / [`crate::director::Phase::Motion`]
//!   translate intent/affect into rendered face + physical motion.
//!   This is the single most important architectural invariant for
//!   NPC composition.
//!
//! [`Face`]: crate::face::Face
//! [`Motor`]: crate::motor::Motor
//! [`Perception`]: crate::perception::Perception
//! [`Voice`]: crate::voice::Voice
//! [`Mind`]: crate::mind::Mind
//! [`Events`]: crate::events::Events

use crate::clock::Instant;
use crate::events::Events;
use crate::face::Face;
use crate::input::Input;
use crate::mind::Mind;
use crate::motor::Motor;
use crate::perception::Perception;
use crate::voice::Voice;

/// Per-frame timing stamped on [`Entity`] by [`crate::Director::run`].
///
/// Modifiers read `entity.tick.now` instead of taking a `now: Instant`
/// argument — keeps the `Modifier::update` signature single-arg and
/// makes `dt_ms` / `frame` available for time-derivative work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Wall time (or simulated time) this frame.
    pub now: Instant,
    /// Milliseconds since the previous `App::run` call. `0` on the first
    /// frame (no previous reference).
    pub dt_ms: u32,
    /// Monotonic frame counter, starting at `1` after the first
    /// `App::run` call. Useful as a cheap change-detection key for
    /// future skills that want "wake me when X changed."
    pub frame: u64,
}

/// The composed entity: a single NPC.
///
/// `Eq` is intentionally not derived because [`Face`] contains an
/// `f32` (`Mouth::mouth_open`) and [`Motor`] contains `Pose`s with
/// `f32` fields. Use `==` (`PartialEq`) for tests; the renderer uses
/// [`Entity::frame_eq`] for its dirty-check (visual fields only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entity {
    /// Visual surface. Read by the renderer.
    pub face: Face,
    /// Physical motion state. Forwarded to head servos.
    pub motor: Motor,
    /// Raw sensor readings. Populated by firmware Signal drains.
    pub perception: Perception,
    /// Speech I/O. Modifiers set `voice.chirp_request` to trigger
    /// firmware audio enqueue.
    pub voice: Voice,
    /// Cognitive layer (affect, autonomy, plus v2.x stubs).
    pub mind: Mind,
    /// Pending inputs the modifier graph consumes (tap edges, IR
    /// pairs). Set by firmware drains; cleared by the consuming
    /// modifier. **Not** cleared by the Director at frame start.
    pub input: Input,
    /// One-frame fire flags. Cleared at frame start by [`crate::Director::run`].
    pub events: Events,
    /// Per-frame timing. Stamped by [`crate::Director::run`].
    pub tick: Tick,
}

impl Default for Entity {
    /// The neutral resting NPC: default face, neutral pose, no sensor
    /// readings yet, no chirps pending, neutral mind, no events fired,
    /// zeroed tick.
    fn default() -> Self {
        Self {
            face: Face::default(),
            motor: Motor::default(),
            perception: Perception::default(),
            voice: Voice::default(),
            input: Input::default(),
            mind: Mind::default(),
            events: Events::default(),
            tick: Tick::default(),
        }
    }
}

impl Entity {
    /// Visual-state equality used by the firmware's render-loop
    /// dirty-check: `true` iff `self` and `other` would render to the
    /// same pixels. Compares only [`Self::face`]; sensor / motor /
    /// mind / voice / events / tick state is excluded — those can
    /// change without producing pixel differences (the LCD is rigidly
    /// mounted to the head, modifiers translate sensor changes into
    /// face changes via emotion, etc.).
    ///
    /// This is dramatically simpler than v0.x's `Avatar::frame_eq`
    /// (which had to enumerate 10 visual fields) — the component
    /// model gives us this for free.
    #[must_use]
    pub fn frame_eq(&self, other: &Self) -> bool {
        self.face == other.face
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::EyePhase;

    #[test]
    fn default_is_neutral() {
        let e = Entity::default();
        assert_eq!(e.mind.affect.emotion, crate::emotion::Emotion::default());
        assert_eq!(e.mind.autonomy.manual_until, None);
        assert_eq!(e.mind.autonomy.source, None);
        assert_eq!(e.face.left_eye.phase, EyePhase::Open);
        assert_eq!(e.tick.frame, 0);
    }

    #[test]
    fn frame_eq_ignores_non_face_state() {
        let mut a = Entity::default();
        let mut b = Entity::default();
        b.motor.head_pose = crate::head::Pose {
            pan_deg: 5.0,
            tilt_deg: 3.0,
        };
        b.perception.battery_percent = Some(42);
        b.tick.frame = 100;
        assert!(a.frame_eq(&b), "non-face state must not affect frame_eq");
        a.face.style.eye_curve = 50;
        assert!(!a.frame_eq(&b), "face state must affect frame_eq");
    }
}
