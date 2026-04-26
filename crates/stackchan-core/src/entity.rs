//! The NPC entity, composed of sub-components.
//!
//! [`Entity`] groups [`Face`] (visual), [`Motor`] (motion),
//! [`Perception`] (sensors), [`Voice`] (speech I/O), [`Mind`] (brain),
//! [`Input`] (pending firmware → modifier inputs), and [`Events`]
//! (one-frame fire flags). [`Tick`] is bookkeeping that
//! [`crate::Director`] stamps each frame.
//!
//! Modifiers can touch any sub-component, but conventionally each
//! modifier writes only the components matching its
//! [`crate::director::Phase`]. Skills don't write `face` or `motor`
//! directly — they write `mind` / `voice` / `events`, and modifiers in
//! [`crate::director::Phase::Expression`] / [`crate::director::Phase::Motion`]
//! translate that intent into rendered face and physical motion.
//!
//! [`Face`]: crate::face::Face
//! [`Motor`]: crate::motor::Motor
//! [`Perception`]: crate::perception::Perception
//! [`Voice`]: crate::voice::Voice
//! [`Mind`]: crate::mind::Mind
//! [`Input`]: crate::input::Input
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
/// Modifiers read `entity.tick.now` instead of taking `now: Instant`
/// as an argument; `dt_ms` and `frame` are available for time-derivative
/// work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Wall (or simulated) time this frame.
    pub now: Instant,
    /// Milliseconds since the previous `Director::run`. `0` on the
    /// first frame.
    pub dt_ms: u32,
    /// Monotonic frame counter, `1` after the first `Director::run`.
    pub frame: u64,
}

/// The composed entity: a single NPC.
///
/// `Eq` is not derived because [`Face`] contains `f32` fields
/// (`Mouth::mouth_open`) and [`Motor`] contains `Pose`s with `f32`
/// fields. `Copy` is not derived either: [`Perception`] holds a
/// `heapless::Vec` of tracker candidates which can't be `Copy`.
/// Use `PartialEq` for tests; the renderer uses [`Entity::frame_eq`]
/// for its dirty-check (visual fields only).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Entity {
    /// Visual surface. Read by the renderer.
    pub face: Face,
    /// Physical motion state. Forwarded to head servos.
    pub motor: Motor,
    /// Raw sensor readings. Populated by firmware Signal drains.
    pub perception: Perception,
    /// Speech I/O. Modifiers set `voice.chirp_request` to trigger an
    /// audio enqueue.
    pub voice: Voice,
    /// Cognitive layer (affect, autonomy).
    pub mind: Mind,
    /// Pending firmware → modifier inputs (tap edges, IR pairs). Set
    /// by firmware drains; cleared by the consuming modifier. Not
    /// cleared by the Director at frame start.
    pub input: Input,
    /// One-frame fire flags. Cleared by [`crate::Director::run`].
    pub events: Events,
    /// Per-frame timing. Stamped by [`crate::Director::run`].
    pub tick: Tick,
}

impl Entity {
    /// Visual-state equality used by the render loop's dirty-check:
    /// `true` iff `self` and `other` would render to the same pixels.
    /// Compares only [`Self::face`]; sensor / motor / mind / voice /
    /// events / tick state is excluded.
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
