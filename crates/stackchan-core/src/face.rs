//! Visual surface of the entity: the rendered face.
//!
//! [`Face`] groups the eye / mouth geometry plus the emotion-driven
//! [`Style`] that shapes how those primitives are drawn. It owns
//! everything the renderer needs to produce a frame; non-visual state
//! (sensors, motor pose, mind/affect, voice queue) lives elsewhere on
//! [`Entity`].
//!
//! The split exists because v0.x packed all of these into a single
//! `Avatar` struct, which made it impossible to express domain
//! boundaries in the type system: a mod that "tweaks the face" had to
//! reach across motor + sensor fields too. With `Face` as a sub-component
//! of [`Entity`], modifiers in [`Phase::Expression`] borrow only the
//! visual surface they actually mutate.
//!
//! All coordinates are in an abstract 320×240 framebuffer space so the
//! domain logic stays resolution-agnostic until the pixel pipeline needs
//! a concrete resolution.
//!
//! [`Entity`]: crate::entity::Entity
//! [`Phase::Expression`]: crate::app::Phase::Expression

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
    /// Ignored by the renderer when [`Style::mouth_curve`] is non-zero.
    pub weight: u8,
    /// Audio-driven mouth-open amount, 0.0..=1.0.
    ///
    /// Written by the `MouthOpenAudio` modifier in response to
    /// microphone input; a value of `0.0` is a closed mouth, `1.0` is
    /// fully open. Additive to [`Self::weight`] / [`Style::mouth_curve`]
    /// at the renderer — emotion keeps its static mouth shape while
    /// talking drives this field for a lip-sync-like effect.
    pub mouth_open: f32,
}

/// Neutral value for a `u8` scale field where 128 = default speed/size.
/// Lower values dampen, higher values amplify. Centralised so tests and
/// the renderer agree on the midpoint.
pub const SCALE_DEFAULT: u8 = 128;

/// Emotion-driven appearance modulators.
///
/// Written by the `EmotionStyle` modifier in [`Phase::Expression`];
/// consumed by the renderer (`Face::draw`) and the `Blink` / `Breath`
/// modifiers (which read the *_scale fields to modulate their cadence).
/// Defaults are chosen so a `Style::default()` renders exactly like
/// v0.1.0 pre-emotion.
///
/// [`Phase::Expression`]: crate::app::Phase::Expression
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Eye curvature, -100..=100. 0 renders a filled ellipse (the v0.1.0
    /// look). Positive = upward arc (smile eyes, Happy). Negative =
    /// downward arc (sad eyes).
    pub eye_curve: i8,
    /// Mouth curvature, -100..=100. 0 defers to [`Mouth::weight`] (line
    /// when 0, filled ellipse otherwise). Positive = smile arc. Negative
    /// = frown arc.
    pub mouth_curve: i8,
    /// Cheek blush intensity, 0..=255. 0 = no cheeks drawn. The renderer
    /// owns the palette mapping.
    pub cheek_blush: u8,
    /// Eye-size scale, 0..=255. [`SCALE_DEFAULT`] (128) = baseline radii.
    /// Surprised raises this to enlarge the eyes.
    pub eye_scale: u8,
    /// Blink-cadence scale, 0..=255. [`SCALE_DEFAULT`] (128) = baseline
    /// timing. 0 suppresses blinks entirely (Surprised holds eyes wide).
    pub blink_rate_scale: u8,
    /// Breath-amplitude scale, 0..=255. [`SCALE_DEFAULT`] (128) =
    /// baseline 2px peak-to-peak. Sleepy deepens this; Surprised reduces
    /// it.
    pub breath_depth_scale: u8,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: SCALE_DEFAULT,
            breath_depth_scale: SCALE_DEFAULT,
        }
    }
}

/// The visual surface of the entity. Owns everything the renderer reads
/// to produce a frame — both the geometric primitives ([`Eye`], [`Mouth`])
/// and the emotion-driven modulators ([`Style`]).
///
/// `Eq` is intentionally not derived because [`Mouth::mouth_open`] is
/// `f32`. Use `==` (`PartialEq`) for tests that need exact comparison;
/// the renderer uses [`crate::entity::Entity::frame_eq`] which delegates
/// here for its dirty-check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    /// Left eye (viewer's left).
    pub left_eye: Eye,
    /// Right eye (viewer's right).
    pub right_eye: Eye,
    /// Mouth.
    pub mouth: Mouth,
    /// Emotion-driven appearance modulators.
    pub style: Style,
}

impl Default for Face {
    /// The neutral resting face: two round eyes + a small mouth, no
    /// emotion-driven modulation. Geometry is tuned for a 320×240
    /// framebuffer.
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
            style: Style::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_face_eyes_symmetric() {
        let f = Face::default();
        // Eyes are mirrored about x = 160 (centre of 320 px wide framebuffer).
        let left_offset = 160 - f.left_eye.center.x;
        let right_offset = f.right_eye.center.x - 160;
        assert_eq!(left_offset, right_offset);
    }

    #[test]
    fn default_style_is_neutral() {
        let s = Style::default();
        assert_eq!(s.eye_curve, 0);
        assert_eq!(s.mouth_curve, 0);
        assert_eq!(s.cheek_blush, 0);
        assert_eq!(s.eye_scale, SCALE_DEFAULT);
        assert_eq!(s.blink_rate_scale, SCALE_DEFAULT);
        assert_eq!(s.breath_depth_scale, SCALE_DEFAULT);
    }
}
