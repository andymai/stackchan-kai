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

use crate::emotion::Emotion;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Neutral value for a `u8` scale field where 128 = default speed/size.
/// Lower values dampen, higher values amplify. Centralized so tests and
/// the renderer agree on the midpoint.
pub const SCALE_DEFAULT: u8 = 128;

/// The composed avatar. Contains two eyes, a mouth, an emotion, and the
/// emotion-driven style fields that renderer + tempo modifiers consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            },
            emotion: Emotion::Neutral,
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: SCALE_DEFAULT,
            breath_depth_scale: SCALE_DEFAULT,
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
