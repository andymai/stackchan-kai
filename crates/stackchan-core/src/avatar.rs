//! Avatar data model: two eyes, a mouth, and an emotion.
//!
//! This module owns the canonical state of the face. Modifiers mutate an
//! `Avatar` directly; renderers read from it. All coordinates are in an
//! abstract 320x240 framebuffer space so the domain logic is
//! resolution-agnostic until the pixel pipeline needs a concrete resolution.

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
    pub weight: u8,
}

/// The composed avatar. Contains two eyes, a mouth, and an emotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Avatar {
    /// Left eye (viewer's left).
    pub left_eye: Eye,
    /// Right eye (viewer's right).
    pub right_eye: Eye,
    /// Mouth.
    pub mouth: Mouth,
    /// Current emotional expression.
    pub emotion: Emotion,
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
            },
            right_eye: Eye {
                center: Point::new(220, 110),
                radius_x: 25,
                radius_y: 25,
                phase: EyePhase::Open,
                weight: 100,
            },
            mouth: Mouth {
                center: Point::new(160, 180),
                radius_x: 32,
                radius_y: 10,
                weight: 0,
            },
            emotion: Emotion::Neutral,
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
    fn eye_height_scales_with_weight() {
        let mut eye = Eye {
            center: Point::new(0, 0),
            radius_x: 25,
            radius_y: 25,
            phase: EyePhase::Open,
            weight: 100,
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
        };
        assert!(eye.height() <= 50);
    }
}
