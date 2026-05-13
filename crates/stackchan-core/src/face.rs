//! Visual surface of the entity: the rendered face.
//!
//! [`Face`] groups the eye and mouth geometry plus the emotion-driven
//! [`Style`] that shapes how those primitives are drawn. It owns
//! everything the renderer needs to produce a frame; non-visual state
//! (sensors, motor pose, mind, voice) lives elsewhere on [`Entity`].
//!
//! All coordinates are in an abstract 320×240 framebuffer space so the
//! domain logic stays resolution-agnostic until the pixel pipeline
//! needs a concrete resolution.
//!
//! [`Entity`]: crate::entity::Entity

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
    /// this on every open transition, so `StyleFromEmotion` can drop it (e.g.
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
    /// Written by the `MouthFromAudio` modifier in response to
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
/// Written by the `StyleFromEmotion` modifier in [`Phase::Expression`];
/// consumed by the renderer (`Face::draw`) and the `Blink` / `Breath`
/// modifiers (which read the *_scale fields to modulate their cadence).
/// Defaults are chosen so a `Style::default()` renders exactly like
/// v0.1.0 pre-emotion.
///
/// [`Phase::Expression`]: crate::director::Phase::Expression
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

/// Eye geometry baseline — the static layout for a preset.
///
/// Holds the values a [`FaceGeometry`] contributes when applied.
/// Dynamic state ([`Eye::phase`], [`Eye::weight`], [`Eye::open_weight`])
/// is preserved across preset swaps and lives only on [`Eye`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EyeBaseline {
    /// Center of the eye in framebuffer space.
    pub center: Point,
    /// Horizontal half-axis in pixels.
    pub radius_x: u16,
    /// Vertical half-axis in pixels.
    pub radius_y: u16,
}

/// Mouth geometry baseline. Dynamic state ([`Mouth::weight`],
/// [`Mouth::mouth_open`]) is preserved across preset swaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouthBaseline {
    /// Center of the mouth in framebuffer space.
    pub center: Point,
    /// Horizontal half-axis in pixels.
    pub radius_x: u16,
    /// Vertical half-axis in pixels.
    pub radius_y: u16,
}

/// Named face-geometry preset.
///
/// Picks one of the curated baseline silhouettes bundled with the
/// firmware. `Default` matches the neutral resting face from v0.1.0
/// so a missing field on disk renders identically.
///
/// Geometry presets are orthogonal to [`crate::palette::Palette`]
/// (skin colours) and [`Style`] (emotion-driven modulators). Picking
/// `Chibi` swaps the eye / mouth *baseline*; emotion still scales
/// eye size, blink cadence, and breath depth on top of that baseline
/// via `Style`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FaceGeometry {
    /// Original v0.1.0 baseline: round eyes at framebuffer-quarter
    /// positions, small mouth.
    #[default]
    Default,
    /// Kawaii silhouette: enlarged eyes, smaller mouth, lower-set.
    Chibi,
    /// Cartoon silhouette: far-set eyes, wider mouth.
    Wide,
    /// Droopy silhouette: lower-set eyes, narrower vertical axis.
    /// Distinct from the `Sleepy` emotion (which modulates `Style`)
    /// — this is a baseline-shape choice the operator pins.
    Sleepy,
}

impl FaceGeometry {
    /// Lowercase wire name for the HTTP `POST /face-geometry` body
    /// and the persisted `RUNTIME.RON` field. Mirrors the convention
    /// from [`crate::palette::Palette::wire_str`].
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Chibi => "chibi",
            Self::Wide => "wide",
            Self::Sleepy => "sleepy",
        }
    }

    /// Single-byte wire encoding for any future BLE GATT exposure.
    /// Append-only; new variants get the next free integer.
    #[must_use]
    pub const fn wire_byte(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Chibi => 1,
            Self::Wide => 2,
            Self::Sleepy => 3,
        }
    }

    /// Parse a lowercase wire string back into a [`FaceGeometry`].
    #[must_use]
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "chibi" => Some(Self::Chibi),
            "wide" => Some(Self::Wide),
            "sleepy" => Some(Self::Sleepy),
            _ => None,
        }
    }

    /// Every variant in declaration order. Manually maintained — the
    /// exhaustive matches in [`Self::wire_str`] / [`Self::wire_byte`]
    /// guard the encoding side; the
    /// `all_length_matches_variant_count` test trips on variant
    /// addition.
    pub const ALL: &'static [Self] = &[Self::Default, Self::Chibi, Self::Wide, Self::Sleepy];

    /// Resolve to the (left eye, right eye, mouth) baseline geometry
    /// for the renderer.
    #[must_use]
    pub const fn baseline(self) -> (EyeBaseline, EyeBaseline, MouthBaseline) {
        match self {
            Self::Default => (
                EyeBaseline {
                    center: Point::new(100, 110),
                    radius_x: 25,
                    radius_y: 25,
                },
                EyeBaseline {
                    center: Point::new(220, 110),
                    radius_x: 25,
                    radius_y: 25,
                },
                MouthBaseline {
                    center: Point::new(160, 180),
                    radius_x: 32,
                    radius_y: 10,
                },
            ),
            Self::Chibi => (
                EyeBaseline {
                    center: Point::new(105, 115),
                    radius_x: 32,
                    radius_y: 32,
                },
                EyeBaseline {
                    center: Point::new(215, 115),
                    radius_x: 32,
                    radius_y: 32,
                },
                MouthBaseline {
                    center: Point::new(160, 195),
                    radius_x: 18,
                    radius_y: 6,
                },
            ),
            Self::Wide => (
                EyeBaseline {
                    center: Point::new(85, 105),
                    radius_x: 22,
                    radius_y: 22,
                },
                EyeBaseline {
                    center: Point::new(235, 105),
                    radius_x: 22,
                    radius_y: 22,
                },
                MouthBaseline {
                    center: Point::new(160, 180),
                    radius_x: 44,
                    radius_y: 12,
                },
            ),
            Self::Sleepy => (
                EyeBaseline {
                    center: Point::new(100, 125),
                    radius_x: 22,
                    radius_y: 14,
                },
                EyeBaseline {
                    center: Point::new(220, 125),
                    radius_x: 22,
                    radius_y: 14,
                },
                MouthBaseline {
                    center: Point::new(160, 188),
                    radius_x: 24,
                    radius_y: 6,
                },
            ),
        }
    }
}

/// Coarse battery-level bucket for the on-screen indicator.
///
/// Quantising the percent reading at the source keeps the dirty-check
/// rare: a per-percent flicker doesn't re-render. Each bucket maps
/// to a distinct glyph (segment count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[non_exhaustive]
pub enum BatteryBucket {
    /// `0..=9` — empty cell, urgent.
    Critical,
    /// `10..=24` — one segment.
    Low,
    /// `25..=49` — two segments.
    Medium,
    /// `50..=74` — three segments.
    High,
    /// `75..=100` — four segments (full).
    #[default]
    Full,
}

impl BatteryBucket {
    /// Map a raw `0..=100` percent reading to its rendered bucket.
    /// Out-of-range inputs saturate to [`Self::Full`] — the AXP2101
    /// gauge can briefly report >100 during a fresh charge, and the
    /// renderer should keep showing a full cell rather than blank.
    #[must_use]
    pub const fn from_percent(p: u8) -> Self {
        match p {
            0..=9 => Self::Critical,
            10..=24 => Self::Low,
            25..=49 => Self::Medium,
            50..=74 => Self::High,
            _ => Self::Full,
        }
    }
}

/// On-screen battery indicator state.
///
/// `None` on [`Face::battery_overlay`] means the renderer skips the
/// indicator. Set by [`crate::modifiers::BatteryOverlayFromPerception`]
/// when the operator has opted in and a battery reading has landed
/// at least once. The bucket field is quantised so frame-to-frame
/// percent jitter doesn't trip the renderer's dirty-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatteryOverlay {
    /// Quantised battery level for rendering.
    pub bucket: BatteryBucket,
    /// `true` while USB power is reported present by the AXP2101.
    pub charging: bool,
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
    /// Active decorator overlay, drawn on top of the base face. `None`
    /// is the steady state. Trigger modifiers in
    /// [`crate::director::Phase::Decoration`] populate this field;
    /// [`crate::modifiers::DecoratorExpiry`] clears it on deadline.
    pub decorator: Option<crate::decorator::DecoratorState>,
    /// Active speech-bubble text overlay, drawn above the face. `None`
    /// is the steady state. Trigger paths (e.g. soliloquy modifier,
    /// firmware-side MCP `speak` short-circuit) populate this field;
    /// [`crate::modifiers::BubbleExpiry`] clears it on deadline.
    pub bubble: Option<crate::bubble::BubbleState>,
    /// On-screen battery indicator, drawn in the top-left corner. `None`
    /// is the steady state (operator has not enabled the overlay, or no
    /// battery reading has landed yet).
    pub battery_overlay: Option<BatteryOverlay>,
    /// Active colour palette. Picks the background / eye / mouth /
    /// cheek colours used by the renderer. Decorator and bubble
    /// overlays keep their own dedicated colours so a palette swap
    /// doesn't desaturate the symbolic-overlay layer's distinctness.
    pub palette: crate::palette::Palette,
    /// Active geometry preset. Picks the baseline eye / mouth layout
    /// from [`FaceGeometry`]. Mutated through [`Face::set_geometry`]
    /// so dynamic state on [`Eye`] / [`Mouth`] (blink phase, mouth
    /// open amount) survives the swap.
    pub geometry: FaceGeometry,
}

impl Face {
    /// Construct a fresh `Face` initialised to the given geometry preset.
    /// Style / decorator / bubble / palette take their default values.
    #[must_use]
    pub fn with_geometry(geometry: FaceGeometry) -> Self {
        let mut face = Self::default();
        face.set_geometry(geometry);
        face
    }

    /// Swap the geometry preset, replacing baseline eye / mouth
    /// dimensions while preserving dynamic state ([`Eye::phase`],
    /// [`Eye::weight`], [`Eye::open_weight`], [`Mouth::weight`],
    /// [`Mouth::mouth_open`]). A mid-blink swap therefore continues
    /// the blink at the new geometry instead of forcing eyes open.
    pub const fn set_geometry(&mut self, geometry: FaceGeometry) {
        let (left, right, mouth) = geometry.baseline();
        self.geometry = geometry;
        self.left_eye.center = left.center;
        self.left_eye.radius_x = left.radius_x;
        self.left_eye.radius_y = left.radius_y;
        self.right_eye.center = right.center;
        self.right_eye.radius_x = right.radius_x;
        self.right_eye.radius_y = right.radius_y;
        self.mouth.center = mouth.center;
        self.mouth.radius_x = mouth.radius_x;
        self.mouth.radius_y = mouth.radius_y;
    }
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
            decorator: None,
            bubble: None,
            battery_overlay: None,
            palette: crate::palette::Palette::Default,
            geometry: FaceGeometry::Default,
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

    #[test]
    fn face_geometry_all_length_matches_variant_count() {
        assert_eq!(
            FaceGeometry::ALL.len(),
            4,
            "update FaceGeometry::ALL when adding a variant",
        );
    }

    #[test]
    fn face_geometry_wire_byte_mapping_is_stable() {
        assert_eq!(FaceGeometry::Default.wire_byte(), 0);
        assert_eq!(FaceGeometry::Chibi.wire_byte(), 1);
        assert_eq!(FaceGeometry::Wide.wire_byte(), 2);
        assert_eq!(FaceGeometry::Sleepy.wire_byte(), 3);
    }

    #[test]
    fn face_geometry_wire_str_round_trip() {
        for &g in FaceGeometry::ALL {
            assert_eq!(FaceGeometry::from_wire_str(g.wire_str()), Some(g));
        }
    }

    #[test]
    fn face_geometry_from_wire_str_rejects_unknown() {
        assert_eq!(FaceGeometry::from_wire_str(""), None);
        assert_eq!(FaceGeometry::from_wire_str("DEFAULT"), None);
        assert_eq!(FaceGeometry::from_wire_str("compact"), None);
    }

    #[test]
    fn face_geometry_default_baseline_matches_legacy_face_default() {
        let baseline = FaceGeometry::Default.baseline();
        let legacy = Face::default();
        assert_eq!(baseline.0.center, legacy.left_eye.center);
        assert_eq!(baseline.0.radius_x, legacy.left_eye.radius_x);
        assert_eq!(baseline.0.radius_y, legacy.left_eye.radius_y);
        assert_eq!(baseline.1.center, legacy.right_eye.center);
        assert_eq!(baseline.2.center, legacy.mouth.center);
        assert_eq!(baseline.2.radius_x, legacy.mouth.radius_x);
        assert_eq!(baseline.2.radius_y, legacy.mouth.radius_y);
    }

    #[test]
    fn face_geometry_baselines_are_pairwise_distinct() {
        for (i, &a) in FaceGeometry::ALL.iter().enumerate() {
            for &b in &FaceGeometry::ALL[i + 1..] {
                assert_ne!(
                    a.baseline(),
                    b.baseline(),
                    "{a:?} and {b:?} share an identical baseline",
                );
            }
        }
    }

    #[test]
    fn set_geometry_preserves_dynamic_state() {
        let mut face = Face::default();
        face.left_eye.phase = EyePhase::Closed;
        face.left_eye.weight = 30;
        face.left_eye.open_weight = 80;
        face.right_eye.weight = 25;
        face.mouth.weight = 50;
        face.mouth.mouth_open = 0.6;

        face.set_geometry(FaceGeometry::Chibi);

        assert_eq!(face.geometry, FaceGeometry::Chibi);
        // Geometry replaced.
        assert_eq!(face.left_eye.center, Point::new(105, 115));
        assert_eq!(face.left_eye.radius_x, 32);
        // Dynamic state untouched.
        assert_eq!(face.left_eye.phase, EyePhase::Closed);
        assert_eq!(face.left_eye.weight, 30);
        assert_eq!(face.left_eye.open_weight, 80);
        assert_eq!(face.right_eye.weight, 25);
        assert_eq!(face.mouth.weight, 50);
        assert!((face.mouth.mouth_open - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn with_geometry_starts_from_default_dynamic_state() {
        let face = Face::with_geometry(FaceGeometry::Wide);
        assert_eq!(face.geometry, FaceGeometry::Wide);
        assert_eq!(face.left_eye.center, Point::new(85, 105));
        assert_eq!(face.left_eye.phase, EyePhase::Open);
        assert_eq!(face.left_eye.weight, 100);
        assert_eq!(face.mouth.weight, 0);
        assert!(face.mouth.mouth_open.abs() < f32::EPSILON);
    }

    #[test]
    fn battery_bucket_boundaries_are_inclusive_low_and_exclusive_high() {
        assert_eq!(BatteryBucket::from_percent(0), BatteryBucket::Critical);
        assert_eq!(BatteryBucket::from_percent(9), BatteryBucket::Critical);
        assert_eq!(BatteryBucket::from_percent(10), BatteryBucket::Low);
        assert_eq!(BatteryBucket::from_percent(24), BatteryBucket::Low);
        assert_eq!(BatteryBucket::from_percent(25), BatteryBucket::Medium);
        assert_eq!(BatteryBucket::from_percent(49), BatteryBucket::Medium);
        assert_eq!(BatteryBucket::from_percent(50), BatteryBucket::High);
        assert_eq!(BatteryBucket::from_percent(74), BatteryBucket::High);
        assert_eq!(BatteryBucket::from_percent(75), BatteryBucket::Full);
        assert_eq!(BatteryBucket::from_percent(100), BatteryBucket::Full);
    }

    #[test]
    fn battery_bucket_out_of_range_saturates_to_full() {
        assert_eq!(BatteryBucket::from_percent(101), BatteryBucket::Full);
        assert_eq!(BatteryBucket::from_percent(u8::MAX), BatteryBucket::Full);
    }

    #[test]
    fn default_face_has_no_battery_overlay() {
        assert!(Face::default().battery_overlay.is_none());
    }

    #[test]
    fn baseline_eye_centers_are_horizontally_symmetric() {
        // Every preset places the eyes mirrored about the
        // 320 px-wide framebuffer's vertical centre line. A typo
        // in any preset's coordinates trips here.
        for &g in FaceGeometry::ALL {
            let (left, right, _) = g.baseline();
            let left_offset = 160 - left.center.x;
            let right_offset = right.center.x - 160;
            assert_eq!(
                left_offset, right_offset,
                "{g:?} eyes are not symmetric about x=160",
            );
            assert_eq!(left.center.y, right.center.y, "{g:?} eyes differ in y");
        }
    }
}
