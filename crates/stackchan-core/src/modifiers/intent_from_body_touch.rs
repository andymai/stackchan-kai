//! `IntentFromBodyTouch`: state-machine gesture recognition on the back-of-head
//! `Si12T` petting strip.
//!
//! Reads `entity.perception.body_touch` (intensity-aware) and emits
//! emotion + autonomy changes for the four gestures M5Stack's
//! reference firmware also recognises:
//!
//! | Gesture        | Trigger                                         | Emotion             |
//! |----------------|-------------------------------------------------|---------------------|
//! | `Press`        | rising edge from no-touch to any-zone touched   | per-zone (mapping)  |
//! | `SwipeForward` | centroid shifts right by ≥ `SWIPE_DELTA` mid-touch | `Happy`          |
//! | `SwipeBackward`| centroid shifts left by ≥ `SWIPE_DELTA` mid-touch  | `Surprised`      |
//! | `Release`      | falling edge to no-touch                        | (no-op)             |
//!
//! State machine mirrors `m5stack/StackChan/firmware/main/hal/hal_head_touch.cpp`:
//! `Idle → Touched (Press) → Swiping (SwipeForward / SwipeBackward) → Idle (Release)`.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::mind::OverrideSource;
use crate::modifier::Modifier;

/// Default emotion set on `Press` of the left zone (no centre touch).
pub const DEFAULT_LEFT_PRESS: Emotion = Emotion::Sleepy;
/// Default emotion set on `Press` of the centre zone (or centre-tied multi-zone).
pub const DEFAULT_CENTRE_PRESS: Emotion = Emotion::Happy;
/// Default emotion set on `Press` of the right zone (no centre touch).
pub const DEFAULT_RIGHT_PRESS: Emotion = Emotion::Surprised;
/// Default emotion set on `SwipeForward` (left → right).
pub const DEFAULT_SWIPE_FORWARD: Emotion = Emotion::Happy;
/// Default emotion set on `SwipeBackward` (right → left).
pub const DEFAULT_SWIPE_BACKWARD: Emotion = Emotion::Surprised;

/// How long any body-touch gesture pins the emotion before
/// `EmotionCycle` is allowed to advance, in milliseconds.
pub const BODY_GESTURE_HOLD_MS: u64 = 5_000;

/// Centroid-shift threshold for swipe detection, in the same
/// `-100..=+100` units `BodyTouch::position()` returns. Matches the
/// upstream reference's `swipe_threshold = 40`.
pub const SWIPE_DELTA: i16 = 40;

/// Per-gesture emotion mapping for [`IntentFromBodyTouch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GestureMapping {
    /// Press of the left zone (no centre).
    pub press_left: Emotion,
    /// Press of the centre zone (or centre-tied multi-zone).
    pub press_centre: Emotion,
    /// Press of the right zone (no centre).
    pub press_right: Emotion,
    /// `SwipeForward` (left → right).
    pub swipe_forward: Emotion,
    /// `SwipeBackward` (right → left).
    pub swipe_backward: Emotion,
}

impl GestureMapping {
    /// Default mapping documented in the module-level table.
    pub const DEFAULT: Self = Self {
        press_left: DEFAULT_LEFT_PRESS,
        press_centre: DEFAULT_CENTRE_PRESS,
        press_right: DEFAULT_RIGHT_PRESS,
        swipe_forward: DEFAULT_SWIPE_FORWARD,
        swipe_backward: DEFAULT_SWIPE_BACKWARD,
    };
}

/// Internal gesture-detection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No touch active.
    Idle,
    /// Touched, no swipe yet. Carries the position recorded at the
    /// initial Press for delta comparison.
    Touched {
        /// Centroid (`-100..=+100`) at the moment of Press; swipes
        /// are detected as deltas from this value.
        initial_position: i16,
    },
    /// Swipe in progress — only one swipe per touch run; subsequent
    /// position changes don't re-fire until release.
    Swiping,
}

/// Gesture-detection modifier for the back-of-head `Si12T` petting strip.
#[derive(Debug, Clone, Copy)]
pub struct IntentFromBodyTouch {
    /// Per-gesture emotion targets.
    pub mapping: GestureMapping,
    /// Hold duration written to `mind.autonomy.manual_until`.
    pub hold_ms: u64,
    /// Centroid-shift threshold for swipe detection.
    pub swipe_delta: i16,
    /// Internal state-machine state.
    state: State,
}

impl IntentFromBodyTouch {
    /// Construct with the default mapping + [`BODY_GESTURE_HOLD_MS`] hold + [`SWIPE_DELTA`] threshold.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mapping: GestureMapping::DEFAULT,
            hold_ms: BODY_GESTURE_HOLD_MS,
            swipe_delta: SWIPE_DELTA,
            state: State::Idle,
        }
    }

    /// Override the per-gesture emotion mapping.
    #[must_use]
    pub const fn with_mapping(mut self, mapping: GestureMapping) -> Self {
        self.mapping = mapping;
        self
    }
}

impl Default for IntentFromBodyTouch {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for IntentFromBodyTouch {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "IntentFromBodyTouch",
            description: "State-machine gesture recognition on perception.body_touch \
                          (Si12T petting strip): Press emits per-zone emotion, \
                          SwipeForward/Backward emit Happy/Surprised. Mirrors M5Stack's \
                          reference hal_head_touch.cpp gesture model.",
            phase: Phase::Affect,
            priority: 0,
            reads: &[Field::BodyTouch],
            writes: &[Field::Emotion, Field::Autonomy],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let Some(touch) = entity.perception.body_touch else {
            return;
        };
        let touched_now = touch.any();

        match (self.state, touched_now) {
            // Idle + no touch = stay idle. Swiping + still touched =
            // already swiped this run, ignore further centroid drift.
            (State::Idle, false) | (State::Swiping, true) => {}
            (State::Idle, true) => {
                // Press: pick zone-derived emotion + transition to Touched.
                let emotion = if touch.centre >= 1 {
                    self.mapping.press_centre
                } else if touch.left >= 1 {
                    self.mapping.press_left
                } else if touch.right >= 1 {
                    self.mapping.press_right
                } else {
                    return;
                };
                pin_emotion(entity, emotion, self.hold_ms);
                self.state = State::Touched {
                    initial_position: touch.position(),
                };
            }
            (State::Touched { initial_position }, true) => {
                // Watch the centroid for a swipe.
                let delta = touch.position() - initial_position;
                if delta >= self.swipe_delta {
                    pin_emotion(entity, self.mapping.swipe_forward, self.hold_ms);
                    self.state = State::Swiping;
                } else if delta <= -self.swipe_delta {
                    pin_emotion(entity, self.mapping.swipe_backward, self.hold_ms);
                    self.state = State::Swiping;
                }
            }
            (State::Touched { .. } | State::Swiping, false) => {
                // Release: return to Idle, no emotion change.
                self.state = State::Idle;
            }
        }
    }
}

/// Write emotion + autonomy hold for any body-touch gesture.
fn pin_emotion(entity: &mut Entity, emotion: Emotion, hold_ms: u64) {
    let now: Instant = entity.tick.now;
    entity.mind.affect.emotion = emotion;
    entity.mind.autonomy.manual_until = Some(now + hold_ms);
    entity.mind.autonomy.source = Some(OverrideSource::BodyTouch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perception::BodyTouch;

    fn entity_with_touch(touch: Option<BodyTouch>) -> Entity {
        let mut e = Entity::default();
        e.perception.body_touch = touch;
        e
    }

    fn run(m: &mut IntentFromBodyTouch, entity: &mut Entity, now_ms: u64) {
        entity.tick.now = Instant::from_millis(now_ms);
        m.update(entity);
    }

    #[test]
    fn no_perception_does_nothing() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(None);
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn press_left_sets_sleepy() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            left: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_LEFT_PRESS);
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::BodyTouch));
    }

    #[test]
    fn press_centre_sets_happy_even_with_other_zones() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            left: 3,
            centre: 3,
            right: 3,
        }));
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_CENTRE_PRESS);
    }

    #[test]
    fn sustained_touch_does_not_re_extend_hold() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        let first_hold = entity.mind.autonomy.manual_until;

        for t in (200..2_000).step_by(100) {
            run(&mut m, &mut entity, t);
        }
        assert_eq!(entity.mind.autonomy.manual_until, first_hold);
    }

    #[test]
    fn release_then_press_fires_again() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            left: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_LEFT_PRESS);

        // Release.
        entity.perception.body_touch = Some(BodyTouch::default());
        run(&mut m, &mut entity, 200);

        // Re-press on the right.
        entity.perception.body_touch = Some(BodyTouch {
            right: 3,
            ..BodyTouch::default()
        });
        run(&mut m, &mut entity, 300);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_RIGHT_PRESS);
    }

    #[test]
    fn left_to_right_slide_fires_swipe_forward() {
        let mut m = IntentFromBodyTouch::new();
        // Press on left.
        let mut entity = entity_with_touch(Some(BodyTouch {
            left: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_LEFT_PRESS);

        // Slide to right — centroid shifts well past +SWIPE_DELTA.
        entity.perception.body_touch = Some(BodyTouch {
            left: 0,
            centre: 0,
            right: 3,
        });
        run(&mut m, &mut entity, 200);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_SWIPE_FORWARD);
    }

    #[test]
    fn right_to_left_slide_fires_swipe_backward() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            right: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_RIGHT_PRESS);

        entity.perception.body_touch = Some(BodyTouch {
            left: 3,
            centre: 0,
            right: 0,
        });
        run(&mut m, &mut entity, 200);
        assert_eq!(entity.mind.affect.emotion, DEFAULT_SWIPE_BACKWARD);
    }

    #[test]
    fn swipe_does_not_re_fire_within_a_run() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            left: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        entity.perception.body_touch = Some(BodyTouch {
            right: 3,
            ..BodyTouch::default()
        });
        run(&mut m, &mut entity, 200);
        let after_swipe = entity.mind.autonomy.manual_until;

        // Continue touching at the new position. No re-fire.
        for t in (300..2_000).step_by(100) {
            run(&mut m, &mut entity, t);
        }
        assert_eq!(entity.mind.autonomy.manual_until, after_swipe);
    }

    #[test]
    fn small_centroid_drift_does_not_trigger_swipe() {
        let mut m = IntentFromBodyTouch::new();
        let mut entity = entity_with_touch(Some(BodyTouch {
            centre: 3,
            ..BodyTouch::default()
        }));
        run(&mut m, &mut entity, 100);
        let after_press = entity.mind.affect.emotion;

        // Slightly stronger right finger pressure — small centroid
        // shift, well under SWIPE_DELTA. No new gesture should fire.
        entity.perception.body_touch = Some(BodyTouch {
            centre: 3,
            right: 1,
            ..BodyTouch::default()
        });
        run(&mut m, &mut entity, 200);
        assert_eq!(entity.mind.affect.emotion, after_press);
    }

    #[test]
    fn body_touch_position_is_centroid() {
        // Pin the math the swipe state machine depends on.
        assert_eq!(
            BodyTouch {
                left: 3,
                ..BodyTouch::default()
            }
            .position(),
            -100,
        );
        assert_eq!(
            BodyTouch {
                right: 3,
                ..BodyTouch::default()
            }
            .position(),
            100,
        );
        assert_eq!(
            BodyTouch {
                centre: 3,
                ..BodyTouch::default()
            }
            .position(),
            0,
        );
        // Equal left + right cancels out.
        assert_eq!(
            BodyTouch {
                left: 2,
                right: 2,
                ..BodyTouch::default()
            }
            .position(),
            0,
        );
        // Position with no touch is zero.
        assert_eq!(BodyTouch::default().position(), 0);
    }
}
