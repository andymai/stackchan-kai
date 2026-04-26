//! `GazeFromAttention`: expression-phase modifier that shifts both
//! eye centers toward the tracked target while
//! [`Attention::Tracking`] is held.
//!
//! Eye gaze is the fastest cue for "the avatar is looking at *you*" —
//! eyes can dart in a single frame, while the head pose has to slew
//! through the `SCServo` control loop. This modifier produces the
//! "eyes lock on first, head catches up" effect that reads as
//! lifelike attention.
//!
//! ## Mapping
//!
//! Linear scale: each degree of head-target pan/tilt maps to
//! [`GAZE_PIXELS_PER_DEG`] pixels of eye-center offset, both eyes
//! shifted equally. Clamped to ±[`GAZE_MAX_OFFSET_PX`] so the iris
//! never escapes the eye outline at extreme tracking targets.
//!
//! ## Composition
//!
//! Diff-and-undo like [`super::IdleDrift`]: subtract our previous
//! offset before adding the new one, so other Expression modifiers
//! that adjust eye centers compose cleanly.

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::Attention;
use crate::modifier::Modifier;
use crate::perception::{HALF_FOV_H_DEG, HALF_FOV_V_DEG};

/// Pixels of eye-center offset per degree of head target pan / tilt.
///
/// `0.5 px/°` plus the [`GAZE_MAX_OFFSET_PX`] clamp puts the eyes at
/// max offset for a ~12° target — comfortable visible response
/// inside the head's working range.
pub const GAZE_PIXELS_PER_DEG: f32 = 0.5;

/// Maximum eye-center offset, in pixels, on either axis.
///
/// Small enough that the iris stays well inside the eye oval at the
/// default radii (`radius_x` ≈ 30 px on QVGA).
pub const GAZE_MAX_OFFSET_PX: i32 = 6;

/// Modifier that translates `mind.attention == Tracking{target}`
/// into an eye-center offset on both eyes.
#[derive(Debug, Clone, Copy)]
pub struct GazeFromAttention {
    /// Pixel offset applied to both eye centers on the previous tick.
    /// Subtracted before applying the new offset (diff-and-undo).
    last_offset: (i32, i32),
}

impl GazeFromAttention {
    /// Construct an idle modifier with no in-flight offset.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_offset: (0, 0),
        }
    }
}

impl Default for GazeFromAttention {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a `(pan_deg, tilt_deg)` head target into a `(dx, dy)`
/// pixel offset, clamped per axis.
#[allow(
    clippy::cast_possible_truncation,
    reason = "input is bounded by tracker pose clamps; result is then \
              explicitly clamped to GAZE_MAX_OFFSET_PX before use"
)]
fn target_to_offset(pan_deg: f32, tilt_deg: f32) -> (i32, i32) {
    let dx = (pan_deg * GAZE_PIXELS_PER_DEG) as i32;
    let dy = (tilt_deg * GAZE_PIXELS_PER_DEG) as i32;
    (
        dx.clamp(-GAZE_MAX_OFFSET_PX, GAZE_MAX_OFFSET_PX),
        dy.clamp(-GAZE_MAX_OFFSET_PX, GAZE_MAX_OFFSET_PX),
    )
}

impl Modifier for GazeFromAttention {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "GazeFromAttention",
            description: "When mind.attention is Tracking{target}, shifts both eye centers \
                          toward the target via GAZE_PIXELS_PER_DEG, clamped to ±GAZE_MAX_OFFSET_PX. \
                          Eye gaze leads head pose for a 'looking at you' effect. Composes \
                          additively after EmotionStyle / Blink / Breath / IdleDrift via diff-and-undo.",
            phase: Phase::Expression,
            // After IdleDrift (priority 0) so the gaze override sits
            // on top of any random gaze jitter.
            priority: 5,
            reads: &[
                Field::Attention,
                Field::LeftEyeCenter,
                Field::RightEyeCenter,
            ],
            writes: &[Field::LeftEyeCenter, Field::RightEyeCenter],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        // Prefer the face centroid over the motion centroid when
        // engaged. The motion target jumps around with frame-
        // differencing noise; the face centroid is steadier and
        // (more importantly) points at the actual face, not at the
        // brightest-changing patch on the body.
        let face_target = entity
            .mind
            .engagement
            .centroid()
            .map(|(nx, ny)| (nx * HALF_FOV_H_DEG, -ny * HALF_FOV_V_DEG));
        let target_offset = match (face_target, entity.mind.attention) {
            (Some((pan, tilt)), _) => target_to_offset(pan, tilt),
            (None, Attention::Tracking { target, .. }) => {
                target_to_offset(target.pan_deg, target.tilt_deg)
            }
            (None, Attention::None | Attention::Listening { .. }) => (0, 0),
        };

        let (prev_x, prev_y) = self.last_offset;
        let (next_x, next_y) = target_offset;

        // Diff-and-undo: subtract our previous offset, add the new
        // one. Both eyes get the same shift (no convergence math).
        let delta_x = next_x - prev_x;
        let delta_y = next_y - prev_y;
        entity.face.left_eye.center.x += delta_x;
        entity.face.left_eye.center.y += delta_y;
        entity.face.right_eye.center.x += delta_x;
        entity.face.right_eye.center.y += delta_y;

        self.last_offset = target_offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;
    use crate::clock::Instant;
    use crate::mind::Engagement;

    fn tracking(pan_deg: f32, tilt_deg: f32) -> Attention {
        Attention::Tracking {
            target: Pose::new(pan_deg, tilt_deg),
            since: Instant::from_millis(0),
        }
    }

    #[test]
    fn no_attention_leaves_eyes_alone() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_left = entity.face.left_eye.center;
        let baseline_right = entity.face.right_eye.center;
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center, baseline_left);
        assert_eq!(entity.face.right_eye.center, baseline_right);
    }

    #[test]
    fn listening_attention_leaves_eyes_alone() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        entity.mind.attention = Attention::Listening {
            since: Instant::from_millis(0),
        };
        let baseline_left = entity.face.left_eye.center;
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center, baseline_left);
    }

    #[test]
    fn tracking_shifts_eyes_toward_target() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;
        let baseline_y = entity.face.left_eye.center.y;

        // Pan +10° (right), tilt +6° (up). At 0.5 px/° that's
        // (5, 3) — both inside the ±6 clamp.
        entity.mind.attention = tracking(10.0, 6.0);
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x + 5);
        assert_eq!(entity.face.left_eye.center.y, baseline_y + 3);
        // Right eye gets the same offset as the left (no convergence
        // math in this modifier). The dedicated `both_eyes_track_same_offset`
        // test below proves the equality more directly.
    }

    #[test]
    fn tracking_offset_clamps_at_max() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;
        // Pan +30° → 15 px raw → clamps to GAZE_MAX_OFFSET_PX.
        entity.mind.attention = tracking(30.0, 0.0);
        m.update(&mut entity);
        assert_eq!(
            entity.face.left_eye.center.x,
            baseline_x + GAZE_MAX_OFFSET_PX
        );
    }

    #[test]
    fn tracking_to_none_returns_eyes_to_baseline() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;

        // Lock onto a target.
        entity.mind.attention = tracking(10.0, 0.0);
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x + 5);

        // Drop attention. Diff-and-undo subtracts last offset.
        entity.mind.attention = Attention::None;
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x);
    }

    #[test]
    fn target_change_updates_offset_in_place() {
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;

        entity.mind.attention = tracking(10.0, 0.0);
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x + 5);

        entity.mind.attention = tracking(-10.0, 0.0);
        m.update(&mut entity);
        assert_eq!(entity.face.left_eye.center.x, baseline_x - 5);
    }

    #[test]
    fn engaged_face_centroid_wins_over_motion_target() {
        // Motion target points one way; face centroid points the
        // other. Eyes must follow the face, not the motion blob.
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;
        // Motion centroid says "look right" (+10° pan).
        entity.mind.attention = tracking(10.0, 0.0);
        // Face is on the LEFT of the frame: centroid.0 = -0.4.
        entity.mind.engagement = Engagement::Locked {
            centroid: (-0.4_f32, 0.0_f32),
            at: Instant::from_millis(0),
        };
        m.update(&mut entity);
        // Face-driven pan = -0.4 × 31° ≈ -12.4° → -6 px (clamped).
        assert!(
            entity.face.left_eye.center.x < baseline_x,
            "eyes should look LEFT toward the face, not RIGHT toward the motion target",
        );
    }

    #[test]
    fn engaged_releasing_state_still_drives_gaze() {
        // Releasing carries the last-known centroid; gaze must keep
        // pointing at it during the search beat.
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let baseline_x = entity.face.left_eye.center.x;
        entity.mind.attention = Attention::None;
        entity.mind.engagement = Engagement::Releasing {
            centroid: (0.5_f32, 0.0_f32),
            at: Instant::from_millis(0),
            misses: 3,
        };
        m.update(&mut entity);
        assert!(entity.face.left_eye.center.x > baseline_x);
    }

    #[test]
    fn both_eyes_track_same_offset() {
        // No convergence math — both eyes see the same delta.
        let mut m = GazeFromAttention::new();
        let mut entity = Entity::default();
        let left_baseline_x = entity.face.left_eye.center.x;
        let right_baseline_x = entity.face.right_eye.center.x;

        entity.mind.attention = tracking(8.0, 0.0);
        m.update(&mut entity);
        let left_delta = entity.face.left_eye.center.x - left_baseline_x;
        let right_delta = entity.face.right_eye.center.x - right_baseline_x;
        assert_eq!(left_delta, right_delta);
    }
}
