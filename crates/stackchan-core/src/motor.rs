//! Physical motion surface of the entity: the head servos.
//!
//! [`Motor`] holds the commanded and observed pose of the pan/tilt head.
//! Modifiers in [`Phase::Motion`] write `head_pose`; the firmware's head
//! task forwards it to the `SCServo` bus and reads back the actual servo
//! position into `head_pose_actual` at ~1 Hz.
//!
//! [`Phase::Motion`]: crate::director::Phase::Motion

use crate::head::Pose;

/// The entity's physical motion state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motor {
    /// Commanded head pose in degrees. Produced by motion modifiers
    /// ([`crate::modifiers::IdleSway`], [`crate::modifiers::EmotionHead`]);
    /// consumed by firmware's head-update task. Excluded from
    /// [`crate::entity::Entity::frame_eq`] — the LCD is rigidly mounted
    /// to the head, so pan/tilt updates never change pixels.
    pub head_pose: Pose,
    /// Observed head pose in degrees — the servos' reported actual
    /// position, not the commanded one. Written by the firmware
    /// head-update task after reading `read_position` from each servo
    /// (~1 Hz). Defaults to [`Pose::NEUTRAL`] and stays there until the
    /// first successful readback.
    pub head_pose_actual: Pose,
}

impl Default for Motor {
    fn default() -> Self {
        Self {
            head_pose: Pose::NEUTRAL,
            head_pose_actual: Pose::NEUTRAL,
        }
    }
}
