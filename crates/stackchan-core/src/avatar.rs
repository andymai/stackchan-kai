//! Avatar data model: two eyes, a mouth, and an emotion.
//!
//! This module owns the canonical state of the face. Modifiers mutate an
//! `Avatar` directly; renderers read from it. All coordinates are in an
//! abstract 320x240 framebuffer space so the domain logic is
//! resolution-agnostic until the pixel pipeline needs a concrete resolution.
//!
//! ## Style fields
//!
//! `Avatar` carries a handful of emotion-driven style fields
//! (`eye_curve`, `mouth_curve`, `cheek_blush`, `eye_scale`,
//! `blink_rate_scale`, `breath_depth_scale`). These are written by the
//! `EmotionStyle` modifier and consumed by the renderer and by the
//! `Blink`/`Breath` modifiers. Defaults are chosen so an `Avatar` with
//! no `EmotionStyle` active renders exactly like v0.1.0 pre-emotion.
//!
//! ## Motion fields
//!
//! `head_pose` carries the current pan/tilt command produced by motion
//! modifiers (e.g. `IdleSway`). It is **not** part of the pixel output —
//! on hardware, the LCD moves with the head, so the face stays centered
//! on screen. Use [`Avatar::frame_eq`] (not `==`) for render-loop dirty
//! checks so pose updates don't force redundant LCD blits.
//!
//! ## Input / autonomy fields
//!
//! `manual_until` is a deadline: while `Some(t)` and the clock is below
//! `t`, autonomous drivers like `EmotionCycle` stand down so the user's
//! explicit input (e.g. a touch-triggered emotion via `EmotionTouch`)
//! sticks. `EmotionTouch::update` clears the field when it expires.
//!
//! ## IMU fields
//!
//! `accel_g` and `gyro_dps` are written by the firmware's IMU task
//! (from raw BMI270 reads) and consumed by motion-reactive modifiers
//! (e.g. `PickupReaction`). They are sensor inputs, not visual state,
//! so they are excluded from [`Avatar::frame_eq`] just like
//! [`Avatar::head_pose`]. Defaults match the resting state of a
//! face-up CoreS3: gravity (`+1 g`) on the Z axis, zero angular rate.
//!
//! ## Ambient light
//!
//! `ambient_lux` is `Some(lux)` after the first successful LTR-553
//! read; `None` beforehand. Consumed by
//! [`super::modifiers::AmbientSleepy`]. Excluded from
//! [`Avatar::frame_eq`].

use crate::clock::Instant;
use crate::emotion::Emotion;
use crate::head::Pose;

/// A 2D integer point in framebuffer space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    /// Horizontal coordinate in pixels.
    pub x: i32,
    /// Vertical coordinate in pixels.
    pub y: i32,
}

impl Point {
    /// Construct a `Point` from `(x, y)` pixel coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Whether an eye is currently open or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EyePhase {
    /// The eye is open (use `weight` to interpolate open amount).
    #[default]
    Open,
    /// The eye is closed (blink).
    Closed,
}

/// A single eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eye {
    /// Center of the eye in framebuffer space.
    pub center: Point,
    /// Horizontal half-axis of the eye oval in pixels.
    pub radius_x: u16,
    /// Vertical half-axis of the eye oval in pixels.
    pub radius_y: u16,
    /// Current open / closed phase.
    pub phase: EyePhase,
    /// Per-frame scale factor for the vertical axis, 0..=100. A `weight` of
    /// 100 uses the full `radius_y`; lower values squish the eye vertically.
    /// The blink modifier drops this toward zero during a blink.
    pub weight: u8,
    /// Upper bound on `weight` when the eye is open, 0..=100. `Blink` reads
    /// this on every open transition, so `EmotionStyle` can drop it (e.g.
    /// `Sleepy = 55`) without fighting Blink's state machine. Default 100.
    pub open_weight: u8,
}

impl Eye {
    /// Width of the eye in pixels at the current weight.
    #[must_use]
    pub const fn width(&self) -> u16 {
        // radius_x * 2 cannot overflow a u16 in practice because the
        // framebuffer itself is 320 px wide; clamp defensively.
        self.radius_x.saturating_mul(2)
    }

    /// Height of the eye in pixels at the current weight.
    #[must_use]
    pub fn height(&self) -> u16 {
        let full = self.radius_y.saturating_mul(2);
        let scaled = u32::from(full) * u32::from(self.weight) / 100;
        #[allow(clippy::cast_possible_truncation)]
        let clamped = scaled.min(u32::from(full)) as u16;
        clamped
    }
}

/// The mouth.
///
/// `Eq` is intentionally not derived: [`Self::mouth_open`] is `f32`,
/// which violates reflexivity on `NaN`. Use the `PartialEq` impl for
/// tests that compare mouth state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mouth {
    /// Center of the mouth in framebuffer space.
    pub center: Point,
    /// Horizontal half-axis of the mouth in pixels.
    pub radius_x: u16,
    /// Vertical half-axis of the mouth in pixels.
    pub radius_y: u16,
    /// Open-amount scale, 0..=100. 0 is a flat line; 100 is fully open.
    /// Ignored by the renderer when `Avatar::mouth_curve` is non-zero.
    pub weight: u8,
    /// Audio-driven mouth-open amount, 0.0..=1.0.
    ///
    /// Written by the `MouthOpenAudio` modifier in response to
    /// microphone input; a value of `0.0` is a closed mouth, `1.0` is
    /// fully open. Additive to [`Self::weight`] / [`super::Avatar::mouth_curve`]
    /// at the renderer — emotion keeps its static mouth shape while
    /// talking drives this field for a lip-sync-like effect. Stays at
    /// `0.0` when the audio subsystem isn't streaming, which renders
    /// as the un-modified emotion mouth (current firmware behaviour).
    pub mouth_open: f32,
}

/// Neutral value for a `u8` scale field where 128 = default speed/size.
/// Lower values dampen, higher values amplify. Centralized so tests and
/// the renderer agree on the midpoint.
pub const SCALE_DEFAULT: u8 = 128;

/// The composed avatar. Contains two eyes, a mouth, an emotion, the
/// emotion-driven style fields that renderer + tempo modifiers consume,
/// and the head [`Pose`] produced by motion modifiers.
///
/// `Eq` is intentionally **not** derived: [`Pose`] uses `f32`, which
/// violates reflexivity for `NaN`. Use [`Avatar::frame_eq`] for the
/// render-loop dirty-check so `head_pose` changes (which don't affect
/// pixels) don't force redundant LCD blits; use `==` (`PartialEq`) for
/// tests that do care about the full state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Avatar {
    /// Left eye (viewer's left).
    pub left_eye: Eye,
    /// Right eye (viewer's right).
    pub right_eye: Eye,
    /// Mouth.
    pub mouth: Mouth,
    /// Current emotional expression. Set by application code (or the demo
    /// `EmotionCycle` modifier); consumed by `EmotionStyle` which translates
    /// it into the style fields below.
    pub emotion: Emotion,
    /// Eye curvature, -100..=100. 0 renders a filled ellipse (the v0.1.0
    /// look). Positive = upward arc (smile eyes, Happy). Negative =
    /// downward arc (sad eyes).
    pub eye_curve: i8,
    /// Mouth curvature, -100..=100. 0 defers to `Mouth::weight` (line when
    /// 0, filled ellipse otherwise). Positive = smile arc. Negative =
    /// frown arc.
    pub mouth_curve: i8,
    /// Cheek blush intensity, 0..=255. 0 = no cheeks drawn. The renderer
    /// owns the palette mapping.
    pub cheek_blush: u8,
    /// Eye-size scale, 0..=255. `SCALE_DEFAULT` (128) = baseline radii.
    /// Surprised raises this to enlarge the eyes.
    pub eye_scale: u8,
    /// Blink-cadence scale, 0..=255. `SCALE_DEFAULT` (128) = baseline
    /// timing. 0 suppresses blinks entirely (Surprised holds eyes wide).
    pub blink_rate_scale: u8,
    /// Breath-amplitude scale, 0..=255. `SCALE_DEFAULT` (128) = baseline
    /// 2px peak-to-peak. Sleepy deepens this; Surprised reduces it.
    pub breath_depth_scale: u8,
    /// Head pan/tilt pose in degrees. Produced by motion modifiers (e.g.
    /// `IdleSway`); consumed by firmware's head-update task, not the
    /// pixel renderer. Excluded from [`Avatar::frame_eq`].
    pub head_pose: Pose,
    /// Observed head pan/tilt pose in degrees — the servos' reported
    /// actual position, not the commanded one. Written by the firmware
    /// head-update task after reading `read_position` from each servo
    /// (at ~1 Hz). Defaults to [`Pose::NEUTRAL`] and stays there until
    /// the first successful readback. Excluded from
    /// [`Avatar::frame_eq`] like [`Avatar::head_pose`] — the LCD
    /// doesn't render against it. A future `EyeGaze` modifier can read
    /// this to point the eyes toward the direction the head is
    /// *actually* facing, decoupled from the command pipeline.
    pub head_pose_actual: Pose,
    /// Deadline until which autonomous emotion drivers should defer to
    /// explicit user input. `None` = autonomy active (default). `Some(t)`
    /// = a user interaction has pinned the current emotion until `t`;
    /// [`super::modifiers::EmotionCycle`] skips advancement while the
    /// deadline is in the future, and
    /// [`super::modifiers::EmotionTouch`] clears the field once it
    /// expires. Excluded from [`Avatar::frame_eq`] — this field only
    /// gates modifier behaviour, not pixels.
    pub manual_until: Option<Instant>,
    /// Accelerometer reading in gravitational units `(x, y, z)`.
    /// Written by the firmware IMU task from raw BMI270 reads at ~100 Hz.
    /// Resting face-up on a flat surface reads `(0, 0, 1)`. Motion-
    /// reactive modifiers (e.g. [`super::modifiers::PickupReaction`])
    /// read this to detect lifts / drops / tilts. Excluded from
    /// [`Avatar::frame_eq`] — IMU updates never affect pixels.
    pub accel_g: (f32, f32, f32),
    /// Gyroscope reading in degrees per second `(x, y, z)`. Written by
    /// the firmware IMU task; consumed by future rotation-reactive
    /// modifiers. Zero at rest. Excluded from [`Avatar::frame_eq`].
    pub gyro_dps: (f32, f32, f32),
    /// Ambient light level in lux, or `None` before the first
    /// successful read. Written by the firmware ambient-light task
    /// (LTR-553); consumed by
    /// [`super::modifiers::AmbientSleepy`]. Excluded from
    /// [`Avatar::frame_eq`] — a dimming room doesn't change pixels
    /// directly; modifiers translate lux into visible state via
    /// `Avatar::emotion`.
    pub ambient_lux: Option<f32>,
    /// Trim-compensated magnetometer reading in microtesla
    /// `(x, y, z)`, or `None` before the first successful BMM150 read.
    /// Written by the firmware magnetometer task; no modifier consumes
    /// it yet (data-only landing). Excluded from [`Avatar::frame_eq`]
    /// — the raw field never affects pixels directly.
    pub mag_ut: Option<(f32, f32, f32)>,
}

impl Avatar {
    /// Visual-state equality: true iff `self` and `other` would render to
    /// the same pixels. Excludes [`Avatar::head_pose`], which is a
    /// kinematic quantity — the LCD is rigidly mounted to the head, so
    /// rotating the head does not move anything on screen.
    ///
    /// The firmware render task uses this as its dirty-check so that
    /// continuous `IdleSway` pose updates don't force redundant blits.
    /// Sim tests that care about full equality (including pose) can use
    /// `==` via [`PartialEq`] instead.
    ///
    /// *Maintenance note:* adding a new pixel-affecting field to
    /// `Avatar` requires extending the comparison below. Non-visual
    /// fields (audio, motor, sensor state) must stay excluded.
    #[must_use]
    pub fn frame_eq(&self, other: &Self) -> bool {
        self.left_eye == other.left_eye
            && self.right_eye == other.right_eye
            && self.mouth == other.mouth
            && self.emotion == other.emotion
            && self.eye_curve == other.eye_curve
            && self.mouth_curve == other.mouth_curve
            && self.cheek_blush == other.cheek_blush
            && self.eye_scale == other.eye_scale
            && self.blink_rate_scale == other.blink_rate_scale
            && self.breath_depth_scale == other.breath_depth_scale
    }
}

impl Default for Avatar {
    /// The `StackChan` default face: two round eyes + a small mouth on a
    /// neutral expression. Geometry is tuned for a 320x240 framebuffer.
    fn default() -> Self {
        Self {
            left_eye: Eye {
                center: Point::new(100, 110),
                radius_x: 25,
                radius_y: 25,
                phase: EyePhase::Open,
                weight: 100,
                open_weight: 100,
            },
            right_eye: Eye {
                center: Point::new(220, 110),
                radius_x: 25,
                radius_y: 25,
                phase: EyePhase::Open,
                weight: 100,
                open_weight: 100,
            },
            mouth: Mouth {
                center: Point::new(160, 180),
                radius_x: 32,
                radius_y: 10,
                weight: 0,
                mouth_open: 0.0,
            },
            emotion: Emotion::Neutral,
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: SCALE_DEFAULT,
            breath_depth_scale: SCALE_DEFAULT,
            head_pose: Pose::NEUTRAL,
            head_pose_actual: Pose::NEUTRAL,
            manual_until: None,
            // Resting face-up: gravity is +1 g along Z, no rotation.
            accel_g: (0.0, 0.0, 1.0),
            gyro_dps: (0.0, 0.0, 0.0),
            // No ambient reading until the LTR-553 task publishes one.
            ambient_lux: None,
            // No magnetometer reading until the BMM150 task publishes one.
            mag_ut: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_avatar_eyes_symmetric() {
        let a = Avatar::default();
        // Eyes are mirrored about x = 160 (center of 320px wide framebuffer).
        let left_offset = 160 - a.left_eye.center.x;
        let right_offset = a.right_eye.center.x - 160;
        assert_eq!(left_offset, right_offset);
    }

    #[test]
    fn default_style_fields_are_neutral() {
        let a = Avatar::default();
        assert_eq!(a.eye_curve, 0);
        assert_eq!(a.mouth_curve, 0);
        assert_eq!(a.cheek_blush, 0);
        assert_eq!(a.eye_scale, SCALE_DEFAULT);
        assert_eq!(a.blink_rate_scale, SCALE_DEFAULT);
        assert_eq!(a.breath_depth_scale, SCALE_DEFAULT);
        assert_eq!(a.left_eye.open_weight, 100);
        assert_eq!(a.right_eye.open_weight, 100);
    }

    #[test]
    fn eye_height_scales_with_weight() {
        let mut eye = Eye {
            center: Point::new(0, 0),
            radius_x: 25,
            radius_y: 25,
            phase: EyePhase::Open,
            weight: 100,
            open_weight: 100,
        };
        assert_eq!(eye.height(), 50);

        eye.weight = 50;
        assert_eq!(eye.height(), 25);

        eye.weight = 0;
        assert_eq!(eye.height(), 0);
    }

    #[test]
    fn default_has_no_manual_override() {
        let a = Avatar::default();
        assert!(
            a.manual_until.is_none(),
            "boot defaults must leave autonomy active"
        );
    }

    #[test]
    fn frame_eq_ignores_manual_until() {
        let a = Avatar::default();
        let mut b = a;
        b.manual_until = Some(Instant::from_millis(1_000));
        assert!(
            a.frame_eq(&b),
            "manual_until is non-visual; frame_eq must skip it"
        );
        assert_ne!(a, b, "PartialEq still sees the difference");
    }

    #[test]
    fn default_imu_is_resting_face_up() {
        let a = Avatar::default();
        assert_eq!(
            a.accel_g,
            (0.0, 0.0, 1.0),
            "default accel must be 1 g on Z so sim tests start at rest",
        );
        assert_eq!(a.gyro_dps, (0.0, 0.0, 0.0));
    }

    #[test]
    fn frame_eq_ignores_imu_fields() {
        let a = Avatar::default();
        let mut b = a;
        b.accel_g = (2.5, -1.0, 0.1);
        b.gyro_dps = (90.0, 0.0, 0.0);
        assert!(
            a.frame_eq(&b),
            "IMU readings are non-visual; frame_eq must skip them",
        );
    }

    #[test]
    fn default_has_no_ambient_reading() {
        assert!(
            Avatar::default().ambient_lux.is_none(),
            "ambient starts unknown until the LTR-553 task publishes",
        );
    }

    #[test]
    fn frame_eq_ignores_ambient_lux() {
        let a = Avatar::default();
        let mut b = a;
        b.ambient_lux = Some(15.0);
        assert!(
            a.frame_eq(&b),
            "ambient reading is non-visual; modifiers translate it, not the renderer",
        );
    }

    #[test]
    fn eye_height_caps_at_100_weight() {
        let eye = Eye {
            center: Point::new(0, 0),
            radius_x: 25,
            radius_y: 25,
            phase: EyePhase::Open,
            weight: 200, // Defensive: should clamp effectively.
            open_weight: 100,
        };
        assert!(eye.height() <= 50);
    }
}
