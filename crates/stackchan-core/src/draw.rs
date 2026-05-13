//! Render a [`Face`] onto any [`DrawTarget`] whose color is [`Rgb565`].
//!
//! The draw code is `no_std`, non-allocating, and hardware-agnostic. The
//! same `Face::draw` call runs against `mipidsi::Display` on the CoreS3
//! and against a `Vec<Rgb565>`-backed framebuffer in `stackchan-sim`'s
//! snapshot tests.
//!
//! ## Palette
//!
//! - Background: `Rgb565::WHITE`.
//! - Eyes: `Rgb565::BLACK`, either filled ellipses (when
//!   [`Style::eye_curve`](crate::face::Style::eye_curve) is 0) or a
//!   stroked polyline arc (otherwise).
//! - Mouth: from `palette.mouth`, either the v0.1.0 line/ellipse (when
//!   [`Style::mouth_curve`](crate::face::Style::mouth_curve) is 0) or
//!   a stroked polyline curve.
//! - Cheeks: a weight-blended white→pink circle below each eye when
//!   [`Style::cheek_blush`](crate::face::Style::cheek_blush) is
//!   non-zero.
//!
//! ## Curves
//!
//! Arcs are drawn as a 17-point polyline sampled from a parabola
//! `y = cy + sag * (1 - u²)`, `u ∈ [-1, 1]`. Integer-only math keeps the
//! code `no_std` without pulling in `libm`; at 320×240 the 17-segment
//! polyline is visually indistinguishable from a continuous curve.

use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::{Point as EgPoint, Size},
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::{Rgb565, RgbColor},
    primitives::{
        Circle, Ellipse, Line, Polyline, Primitive, PrimitiveStyle, PrimitiveStyleBuilder,
        Rectangle,
    },
    text::{Alignment, Baseline, Text, TextStyleBuilder},
};

use crate::bubble::BubbleState;
use crate::decorator::{Decorator, DecoratorState};
use crate::face::{BatteryBucket, BatteryOverlay, Eye, EyePhase, Face, Mouth, SCALE_DEFAULT};
use crate::palette::PaletteColors;

/// Heart decorator pink — slightly more saturated than the `Default`
/// palette's mouth pink so the heart reads as a deliberate overlay
/// rather than blush bleed-through.
const HEART_COLOR: Rgb565 = Rgb565::new(31, 16, 12);

/// Sweat decorator light blue — distinct from any other palette entry
/// so it doesn't cross-talk with the LCD background or the angry/sad
/// face rendering.
const SWEAT_COLOR: Rgb565 = Rgb565::new(8, 32, 28);

/// Dizzy decorator dot color — black so it reads against the white
/// background even at small sizes.
const DIZZY_COLOR: Rgb565 = Rgb565::BLACK;

/// Pairing decorator color — saturated blue, the universal "transmitting"
/// cue. Distinct from sweat-blue so the two never read as the same overlay.
const PAIRING_COLOR: Rgb565 = Rgb565::new(4, 18, 31);

/// Angry decorator color — vivid red, distinct from heart pink and from
/// any cheek-blush warmth so the # symbol reads as deliberate
/// vein-pop rather than emotional flush.
const ANGRY_COLOR: Rgb565 = Rgb565::new(31, 8, 4);

// Shy decorator reuses `EAR_COLOR` (defined alongside the ear
// rendering) so the embarrassed crosshatch reads as part of the
// avatar's blush-family palette rather than a foreign overlay. No
// dedicated constant: drift between two "same warm pink" colours
// would be a real bug a future palette tweak could introduce.

/// Stroke width for closed-eye line, resting mouth line, and curved arcs.
const LINE_WIDTH: u32 = 3;

/// Stroke width for curved eyes (when `eye_curve != 0`). Slightly thicker
/// so a ~50 px wide arc reads as strong as the filled-ellipse variant.
const EYE_ARC_WIDTH: u32 = 5;

/// Number of polyline segments used to approximate one parabolic arc.
/// 16 segments (17 points) keeps the polyline well under embedded-graphics'
/// scanline-iterator limits while reading as a smooth curve at 320×240.
const ARC_SEGMENTS: i32 = 16;

/// Cheek circle diameter, in pixels.
const CHEEK_DIAMETER: u32 = 18;

/// Vertical gap between the bottom of an eye and the top of its cheek.
const CHEEK_VERTICAL_GAP: i32 = 6;

impl Face {
    /// Render `self` onto `target`, clearing the background first.
    ///
    /// # Errors
    ///
    /// Returns any error the underlying `DrawTarget` produces while writing
    /// pixels. This function itself never allocates and never panics.
    pub fn draw<D>(&self, target: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let palette = self.palette.colors();
        target.clear(palette.background)?;
        // Cheeks first: the eye sits on top of the cheek circle when the
        // two overlap at high `eye_scale` + `cheek_blush`.
        if self.style.cheek_blush > 0 {
            draw_cheek(
                &self.left_eye,
                self.style.cheek_blush,
                self.style.eye_scale,
                palette,
                target,
            )?;
            draw_cheek(
                &self.right_eye,
                self.style.cheek_blush,
                self.style.eye_scale,
                palette,
                target,
            )?;
        }
        draw_eye(
            &self.left_eye,
            self.style.eye_curve,
            self.style.eye_scale,
            palette.eye,
            target,
        )?;
        draw_eye(
            &self.right_eye,
            self.style.eye_curve,
            self.style.eye_scale,
            palette.eye,
            target,
        )?;
        draw_mouth(&self.mouth, self.style.mouth_curve, palette.mouth, target)?;
        if let Some(state) = self.decorator {
            draw_decorator(state, target)?;
        }
        if let Some(state) = self.bubble {
            draw_bubble(state, target)?;
        }
        if let Some(overlay) = self.battery_overlay {
            draw_battery(overlay, target)?;
        }
        Ok(())
    }
}

/// Dispatch on [`Decorator`] kind to the per-shape draw routine. Only
/// runs when `face.decorator` is `Some` — `None` is the steady state
/// and short-circuits in `Face::draw`.
fn draw_decorator<D>(state: DecoratorState, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    match state.kind {
        Decorator::Heart => draw_heart(target),
        Decorator::Sweat => draw_sweat(target),
        Decorator::Dizzy => draw_dizzy(target),
        Decorator::Ear => draw_ear(target),
        Decorator::Pairing => draw_pairing(target),
        Decorator::Angry => draw_angry(target),
        Decorator::Shy => draw_shy(target),
    }
}

/// Heart decorator anchor — upper-right of the face. Two overlapping
/// pink circles form the lobes; a small filled triangle fills the
/// bottom point. Anchored at fixed coordinates rather than relative to
/// the eyes because all three decorators share an anchor convention.
const HEART_ANCHOR_X: i32 = 270;
/// Heart decorator anchor Y — see [`HEART_ANCHOR_X`].
const HEART_ANCHOR_Y: i32 = 50;
/// Lobe diameter (passed directly to `Circle::new`). Sized so lobe
/// centres sit `HEART_LOBE_CENTER_OFFSET` apart and the right edge of
/// the left lobe overlaps the left edge of the right lobe by a few
/// pixels — the classic double-bump silhouette.
const HEART_LOBE_DIAMETER: u32 = 12;
/// Horizontal distance from the anchor to each lobe's centre. Picked
/// so the two lobes overlap (centre offset < diameter) for the heart
/// silhouette rather than reading as two isolated dots.
const HEART_LOBE_CENTER_OFFSET: i32 = 4;

/// Draw a small pink heart in the upper-right corner of the face.
///
/// The heart is two overlapping circles (left and right lobes) plus a
/// downward-pointing triangle for the bottom point. Integer math; no
/// floats; non-allocating.
fn draw_heart<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    use embedded_graphics::primitives::Triangle;

    // `Circle::new(top_left, diameter)` — the corner of the bounding
    // box, not the centre. Convert from each lobe's centre to its
    // bounding-box top-left.
    #[allow(clippy::cast_possible_wrap)]
    let half_d = (HEART_LOBE_DIAMETER / 2) as i32;
    let left_center_x = HEART_ANCHOR_X - HEART_LOBE_CENTER_OFFSET;
    let right_center_x = HEART_ANCHOR_X + HEART_LOBE_CENTER_OFFSET;
    let lobe_top_y = HEART_ANCHOR_Y;
    let left_top = EgPoint::new(left_center_x - half_d, lobe_top_y);
    let right_top = EgPoint::new(right_center_x - half_d, lobe_top_y);
    Circle::new(left_top, HEART_LOBE_DIAMETER)
        .into_styled(fill(HEART_COLOR))
        .draw(target)?;
    Circle::new(right_top, HEART_LOBE_DIAMETER)
        .into_styled(fill(HEART_COLOR))
        .draw(target)?;
    // Triangle filling the bottom point. Top corners sit on the lobe
    // bottoms (diameter px below the lobe top); bottom apex sits a
    // further ~diameter below for a balanced heart silhouette.
    #[allow(clippy::cast_possible_wrap)]
    let d = HEART_LOBE_DIAMETER as i32;
    let lobe_bottom_y = lobe_top_y + d;
    Triangle::new(
        EgPoint::new(left_center_x - half_d, lobe_bottom_y - 1),
        EgPoint::new(right_center_x + half_d, lobe_bottom_y - 1),
        EgPoint::new(HEART_ANCHOR_X, lobe_bottom_y + d),
    )
    .into_styled(fill(HEART_COLOR))
    .draw(target)
}

/// Sweat decorator anchor — same upper-right region as Heart since
/// only one decorator shows at a time.
const SWEAT_ANCHOR_X: i32 = 270;
/// Sweat decorator anchor Y — see [`SWEAT_ANCHOR_X`].
const SWEAT_ANCHOR_Y: i32 = 40;

/// Draw a small light-blue sweat drop. Approximated as a vertical
/// ellipse with a small triangle on top (the drop's pointed end).
fn draw_sweat<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    use embedded_graphics::primitives::Triangle;

    // Body: vertical ellipse, 12 px wide × 18 px tall.
    let body_width: u32 = 12;
    let body_height: u32 = 18;
    #[allow(clippy::cast_possible_wrap)]
    let half_w = (body_width / 2) as i32;
    let body_top = EgPoint::new(SWEAT_ANCHOR_X - half_w, SWEAT_ANCHOR_Y + 6);
    Ellipse::new(body_top, Size::new(body_width, body_height))
        .into_styled(fill(SWEAT_COLOR))
        .draw(target)?;
    // Pointed tip: triangle above the ellipse, ~6 px tall.
    Triangle::new(
        EgPoint::new(SWEAT_ANCHOR_X - 4, SWEAT_ANCHOR_Y + 6),
        EgPoint::new(SWEAT_ANCHOR_X + 4, SWEAT_ANCHOR_Y + 6),
        EgPoint::new(SWEAT_ANCHOR_X, SWEAT_ANCHOR_Y),
    )
    .into_styled(fill(SWEAT_COLOR))
    .draw(target)
}

/// Dizzy decorator: three black dots in an arc above the centre of the
/// face. Centred at x=160 (frame midpoint).
const DIZZY_CENTER_X: i32 = 160;
/// Dizzy decorator anchor Y — above the eyes (eye centres are at y=110).
const DIZZY_CENTER_Y: i32 = 30;
/// Diameter of one dizzy dot.
const DIZZY_DOT_DIAMETER: u32 = 8;

/// Draw three small black dots in an arc above the eyes — the
/// stylised "I'm seeing stars" overlay.
fn draw_dizzy<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[allow(clippy::cast_possible_wrap)]
    let half = (DIZZY_DOT_DIAMETER / 2) as i32;
    // Dots at x = -30, 0, +30 from centre. Arc curvature: middle dot
    // sits 4 px lower than the side dots so the trio reads as an
    // arc rather than a straight line.
    let positions: [(i32, i32); 3] = [
        (DIZZY_CENTER_X - 30, DIZZY_CENTER_Y),
        (DIZZY_CENTER_X, DIZZY_CENTER_Y + 4),
        (DIZZY_CENTER_X + 30, DIZZY_CENTER_Y),
    ];
    for (cx, cy) in positions {
        let top_left = EgPoint::new(cx - half, cy - half);
        Circle::new(top_left, DIZZY_DOT_DIAMETER)
            .into_styled(fill(DIZZY_COLOR))
            .draw(target)?;
    }
    Ok(())
}

/// Ear decorator: a small cupped-hand / "C" shape above the
/// upper-left eye, mirroring the head-tilt-and-listen pose. Two
/// concentric arcs (outer + inner) drawn with stacked filled circles
/// form the cup; the contrast between fill colours gives the cupped
/// reading without needing a stroke primitive.
const EAR_ANCHOR_X: i32 = 50;
/// Ear decorator anchor Y — symmetric with Heart (which anchors at
/// upper-right `y = 50`), so the pair would balance if both fired
/// (they shouldn't in practice; mutual-exclusion is enforced by
/// `face.decorator` carrying only one variant at a time).
const EAR_ANCHOR_Y: i32 = 50;
/// Outer-arc diameter for the ear cup.
const EAR_OUTER_DIAMETER: u32 = 22;
/// Inner-arc diameter for the cup hollow. Must be smaller than
/// [`EAR_OUTER_DIAMETER`].
const EAR_INNER_DIAMETER: u32 = 12;
/// Outer ring colour — same warm pink the cheeks already use, so
/// the overlay reads as part of the avatar palette rather than a
/// foreign UI element.
const EAR_COLOR: Rgb565 = Rgb565::new(31, 50, 22);

/// Draw the listening overlay. Two concentric circles (outer pink,
/// inner white) — the white inner circle "subtracts" from the pink
/// outer to leave a ring that reads as an ear / listen indicator.
fn draw_ear<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[allow(clippy::cast_possible_wrap)]
    let outer_half = (EAR_OUTER_DIAMETER / 2) as i32;
    #[allow(clippy::cast_possible_wrap)]
    let inner_half = (EAR_INNER_DIAMETER / 2) as i32;
    let outer_top_left = EgPoint::new(EAR_ANCHOR_X - outer_half, EAR_ANCHOR_Y - outer_half);
    let inner_top_left = EgPoint::new(EAR_ANCHOR_X - inner_half, EAR_ANCHOR_Y - inner_half);
    Circle::new(outer_top_left, EAR_OUTER_DIAMETER)
        .into_styled(fill(EAR_COLOR))
        .draw(target)?;
    Circle::new(inner_top_left, EAR_INNER_DIAMETER)
        .into_styled(fill(Rgb565::WHITE))
        .draw(target)?;
    Ok(())
}

/// Pairing decorator anchor X — top-centre, mirrors `DIZZY_CENTER_X`.
const PAIRING_CENTER_X: i32 = 160;
/// Pairing decorator anchor Y — same band as the dizzy/listening cues.
const PAIRING_CENTER_Y: i32 = 30;
/// Centre dot diameter.
const PAIRING_CORE_DIAMETER: u32 = 6;
/// Inner ring diameter.
const PAIRING_RING_INNER_DIAMETER: u32 = 16;
/// Outer ring diameter.
const PAIRING_RING_OUTER_DIAMETER: u32 = 26;
/// Stroke width on the rings — thick enough to read at 320×240.
const PAIRING_RING_STROKE: u32 = 2;

/// Draw a wireless-signal radiating-from-point glyph: a filled centre
/// dot with two concentric ring outlines. Reads as "transmitting" /
/// "pairing window open" — pulse animation belongs to the LED ring,
/// not the LCD overlay (the avatar's own attention should stay readable
/// during pairing).
fn draw_pairing<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let core_top_left = EgPoint::new(
        #[allow(clippy::cast_possible_wrap)]
        {
            PAIRING_CENTER_X - (PAIRING_CORE_DIAMETER / 2) as i32
        },
        #[allow(clippy::cast_possible_wrap)]
        {
            PAIRING_CENTER_Y - (PAIRING_CORE_DIAMETER / 2) as i32
        },
    );
    Circle::new(core_top_left, PAIRING_CORE_DIAMETER)
        .into_styled(fill(PAIRING_COLOR))
        .draw(target)?;
    for diameter in [PAIRING_RING_INNER_DIAMETER, PAIRING_RING_OUTER_DIAMETER] {
        let top_left = EgPoint::new(
            #[allow(clippy::cast_possible_wrap)]
            {
                PAIRING_CENTER_X - (diameter / 2) as i32
            },
            #[allow(clippy::cast_possible_wrap)]
            {
                PAIRING_CENTER_Y - (diameter / 2) as i32
            },
        );
        Circle::new(top_left, diameter)
            .into_styled(stroke(PAIRING_COLOR, PAIRING_RING_STROKE))
            .draw(target)?;
    }
    Ok(())
}

/// Angry decorator anchor — upper-left, mirroring [`HEART_ANCHOR_X`]
/// (upper-right) so the two never visually collide.
const ANGRY_ANCHOR_X: i32 = 50;
/// Angry decorator anchor Y — same band as Heart so the overlay
/// position reads as a fixed "upper corner cue" regardless of which
/// kind is active.
const ANGRY_ANCHOR_Y: i32 = 48;
/// Half-extent of the `#` symbol's bounding box, in pixels. Each
/// stroke spans `2 × ANGRY_HALF_EXTENT` and the inner crossbars
/// sit at `± ANGRY_BAR_OFFSET` from the centre.
const ANGRY_HALF_EXTENT: i32 = 9;
/// Distance from the anchor centre to each crossbar, in pixels.
const ANGRY_BAR_OFFSET: i32 = 4;
/// Stroke width for each leg of the `#` symbol.
const ANGRY_STROKE: u32 = 3;

/// Draw a red `#` symbol at the upper-left — two short horizontal
/// bars and two short vertical bars, the anime "vein-pop" cue for
/// strong anger. Pure rectangle fills; no curves, no allocations.
fn draw_angry<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // Each bar is rendered as a 1-D rectangle (full width × stroke
    // height for horizontals, stroke width × full height for verticals).
    // Using filled rectangles keeps the path closer to embedded-graphics'
    // fast scanline path than stroked lines.
    let len = ANGRY_HALF_EXTENT * 2;
    #[allow(clippy::cast_sign_loss)]
    let len_u = len as u32;
    let stroke_w = ANGRY_STROKE;
    let half_stroke = i32::try_from(stroke_w / 2).unwrap_or(1);

    // Two horizontal bars — above and below the centre.
    for dy in [-ANGRY_BAR_OFFSET, ANGRY_BAR_OFFSET] {
        let top_left = EgPoint::new(
            ANGRY_ANCHOR_X - ANGRY_HALF_EXTENT,
            ANGRY_ANCHOR_Y + dy - half_stroke,
        );
        Rectangle::new(top_left, Size::new(len_u, stroke_w))
            .into_styled(fill(ANGRY_COLOR))
            .draw(target)?;
    }
    // Two vertical bars — left and right of the centre.
    for dx in [-ANGRY_BAR_OFFSET, ANGRY_BAR_OFFSET] {
        let top_left = EgPoint::new(
            ANGRY_ANCHOR_X + dx - half_stroke,
            ANGRY_ANCHOR_Y - ANGRY_HALF_EXTENT,
        );
        Rectangle::new(top_left, Size::new(stroke_w, len_u))
            .into_styled(fill(ANGRY_COLOR))
            .draw(target)?;
    }
    Ok(())
}

/// Shy decorator: short pink hash marks under each cheek — the
/// embarrassed crosshatch trope, paired left + right so the cue
/// reads as a single bilateral overlay rather than a one-sided dot.
/// X anchors mirror the natural cheek positions; Y sits below the
/// existing cheek-blush band so the overlay augments rather than
/// replaces the base blush.
const SHY_LEFT_ANCHOR_X: i32 = 80;
/// Shy decorator right-cheek anchor X. Mirror of [`SHY_LEFT_ANCHOR_X`]
/// across the frame midline.
const SHY_RIGHT_ANCHOR_X: i32 = 240;
/// Shy decorator anchor Y — sits below the cheek-blush band
/// (cheek circles are centred ~135 with `CHEEK_DIAMETER` 18, so
/// their bottom edge is ~144).
const SHY_ANCHOR_Y: i32 = 175;
/// Length of one shy hash mark, in pixels.
const SHY_MARK_LEN: u32 = 12;
/// Stroke height of one shy hash mark, in pixels.
const SHY_MARK_THICKNESS: u32 = 2;
/// Vertical spacing between the three stacked hash marks.
const SHY_MARK_SPACING: i32 = 4;

/// Draw the shy / embarrassed overlay: three short pink horizontal
/// hash marks under each cheek, stacked vertically.
fn draw_shy<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[allow(clippy::cast_possible_wrap)]
    let half_len = (SHY_MARK_LEN / 2) as i32;
    for anchor_x in [SHY_LEFT_ANCHOR_X, SHY_RIGHT_ANCHOR_X] {
        for offset_idx in [-1_i32, 0, 1] {
            let y = SHY_ANCHOR_Y + offset_idx * SHY_MARK_SPACING;
            let top_left = EgPoint::new(anchor_x - half_len, y);
            Rectangle::new(top_left, Size::new(SHY_MARK_LEN, SHY_MARK_THICKNESS))
                .into_styled(fill(EAR_COLOR))
                .draw(target)?;
        }
    }
    Ok(())
}

/// Speech-bubble outer-rect colour — black border around the
/// white text background. Reads as a clean callout against the
/// white face background.
const BUBBLE_BORDER_COLOR: Rgb565 = Rgb565::BLACK;
/// Speech-bubble fill colour — slightly off-white so the bubble
/// reads as a separate layer, not a hole punched through the face.
/// `Rgb565::new(28, 56, 28)` quantises to roughly `#E6E3E6` after the
/// (5,6,5)-bit channel scaling — a barely perceptible lavender-tinged
/// neutral that does not compete with the avatar palette.
const BUBBLE_FILL_COLOR: Rgb565 = Rgb565::new(28, 56, 28);
/// Speech-bubble text colour.
const BUBBLE_TEXT_COLOR: Rgb565 = Rgb565::BLACK;
/// Speech-bubble border stroke width.
const BUBBLE_BORDER_STROKE: u32 = 2;
/// Vertical padding inside the bubble (top + bottom of text area).
const BUBBLE_VERTICAL_PADDING: i32 = 6;
/// Horizontal padding inside the bubble (left + right of text).
const BUBBLE_HORIZONTAL_PADDING: i32 = 8;
/// Speech-bubble anchor Y — top of the bubble, just below the
/// frame's top edge. Anchors at the top so the bubble doesn't
/// occlude the eyes (which sit at y≈110).
const BUBBLE_ANCHOR_Y: i32 = 4;
/// Framebuffer width assumption used for centring. The render target
/// is the same 320×240 LCD canvas as the rest of the avatar
/// (matches `crates/stackchan-firmware/src/framebuffer.rs::WIDTH`).
const BUBBLE_FB_WIDTH: i32 = 320;
/// Frame-edge clearance — how close the bubble can come to the
/// left / right edge of the framebuffer. Prevents the border from
/// landing on the screen-edge pixel column.
const BUBBLE_EDGE_CLEARANCE: i32 = 4;
/// Maximum bubble width, in pixels. Covers the full top of the frame
/// minus the edge clearance on both sides.
const BUBBLE_MAX_WIDTH: i32 = BUBBLE_FB_WIDTH - 2 * BUBBLE_EDGE_CLEARANCE;
/// `FONT_10X20` glyph width — used to measure rendered text width
/// without round-tripping through `embedded_graphics`'s text
/// metrics API.
const BUBBLE_GLYPH_WIDTH: i32 = 10;
/// `FONT_10X20` glyph height — see [`BUBBLE_GLYPH_WIDTH`].
const BUBBLE_GLYPH_HEIGHT: i32 = 20;

/// Compute the bubble's drawn rectangle in framebuffer coordinates,
/// given the text length in characters. Returns `(top_left, size)`.
/// Width is text-derived but clamped to [`BUBBLE_MAX_WIDTH`] so a
/// runaway-length text never overflows the frame; the renderer
/// truncates the visible characters at the same boundary.
const fn bubble_rect(char_count: usize) -> (EgPoint, Size) {
    let chars = if char_count == 0 { 1 } else { char_count };
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    let raw_text_w = chars as i32 * BUBBLE_GLYPH_WIDTH;
    let max_text_w = BUBBLE_MAX_WIDTH - 2 * BUBBLE_HORIZONTAL_PADDING;
    let text_w = if raw_text_w > max_text_w {
        max_text_w
    } else {
        raw_text_w
    };
    let total_w = text_w + 2 * BUBBLE_HORIZONTAL_PADDING;
    let total_h = BUBBLE_GLYPH_HEIGHT + 2 * BUBBLE_VERTICAL_PADDING;
    // Centred horizontally on the framebuffer.
    let top_left_x = (BUBBLE_FB_WIDTH - total_w) / 2;
    #[allow(clippy::cast_sign_loss)]
    let size = Size::new(total_w as u32, total_h as u32);
    (EgPoint::new(top_left_x, BUBBLE_ANCHOR_Y), size)
}

/// Draw the speech bubble: a bordered rounded-rect-style callout
/// (rendered as a regular rectangle for `no_std`-friendly simplicity)
/// with the bubble text rendered in `FONT_10X20` inside it.
fn draw_bubble<D>(state: BubbleState, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // Truncate at the maximum visible character count for the bubble
    // width. The bubble never word-wraps; firmware feeds short
    // phrase-pool strings.
    let max_visible_chars =
        ((BUBBLE_MAX_WIDTH - 2 * BUBBLE_HORIZONTAL_PADDING) / BUBBLE_GLYPH_WIDTH).max(1);
    #[allow(clippy::cast_sign_loss)]
    let visible_char_cap = max_visible_chars as usize;
    let visible_text = if state.text.chars().count() > visible_char_cap {
        // Byte-truncate at a char boundary corresponding to the cap.
        // `char_indices().nth(cap)` yields the (byte, char) of the
        // *next* char past the cap — its byte index is exactly where
        // we slice. Falls back to the full string when `nth` is None,
        // i.e. when the text has fewer chars than the cap (defensive;
        // the outer length check already short-circuits this path).
        let split = state
            .text
            .char_indices()
            .nth(visible_char_cap)
            .map_or(state.text.len(), |(idx, _)| idx);
        &state.text[..split]
    } else {
        state.text
    };
    let visible_chars = visible_text.chars().count();

    let (top_left, size) = bubble_rect(visible_chars);

    // Filled background.
    Rectangle::new(top_left, size)
        .into_styled(fill(BUBBLE_FILL_COLOR))
        .draw(target)?;
    // Border on top.
    Rectangle::new(top_left, size)
        .into_styled(stroke(BUBBLE_BORDER_COLOR, BUBBLE_BORDER_STROKE))
        .draw(target)?;

    // Text centre point: horizontal midpoint of the bubble, vertical
    // midpoint of the inner area (which sits one glyph-height tall
    // inside the padded rect).
    #[allow(clippy::cast_possible_wrap)]
    let center_x = top_left.x + (size.width / 2) as i32;
    let baseline_y = top_left.y + BUBBLE_VERTICAL_PADDING + BUBBLE_GLYPH_HEIGHT / 2;

    let character_style = MonoTextStyle::new(&FONT_10X20, BUBBLE_TEXT_COLOR);
    let text_style = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();
    Text::with_text_style(
        visible_text,
        EgPoint::new(center_x, baseline_y),
        character_style,
        text_style,
    )
    .draw(target)?;
    Ok(())
}

/// Draw one eye. Decision tree:
///
/// 1. Closed phase, or `weight == 0`: horizontal closed-eye line (unchanged
///    v0.1.0 behavior; curves don't apply when the lid is shut).
/// 2. `curve == 0`: filled ellipse, with radii scaled by `eye_scale`.
/// 3. Otherwise: a stroked parabolic arc. `curve > 0` (Happy) arches
///    upward, `curve < 0` (Sad) dips downward.
#[allow(clippy::similar_names)] // `scaled_rx` / `scaled_ry` is the intended x/y pair.
fn draw_eye<D>(
    eye: &Eye,
    curve: i8,
    scale: u8,
    eye_color: Rgb565,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let scaled_rx = scale_radius(eye.radius_x, scale);
    let scaled_ry = scale_radius(eye.radius_y, scale);
    let height = scaled_height(scaled_ry, eye.weight);

    if matches!(eye.phase, EyePhase::Closed) || height == 0 {
        return draw_horizontal_line(
            eye.center.x,
            eye.center.y,
            scaled_rx,
            stroke(eye_color, LINE_WIDTH),
            target,
        );
    }

    if curve == 0 {
        let width = scaled_rx.saturating_mul(2);
        let half_w = i32::from(width / 2);
        let half_h = i32::from(height / 2);
        let top_left = EgPoint::new(eye.center.x - half_w, eye.center.y - half_h);
        let size = Size::new(u32::from(width), u32::from(height));
        return Ellipse::new(top_left, size)
            .into_styled(fill(eye_color))
            .draw(target);
    }

    // Curved eye: a parabolic arc whose sag is proportional to |curve|
    // and the scaled vertical radius. `curve > 0` (Happy) lifts the
    // middle upward — the inverse sign convention of `mouth_curve`.
    let sag = -i32::from(curve) * i32::from(scaled_ry) / 100;
    draw_parabolic_arc(
        eye.center.x,
        eye.center.y,
        scaled_rx,
        sag,
        stroke(eye_color, EYE_ARC_WIDTH),
        target,
    )
}

/// Maximum pixel height the audio-driven `mouth_open` can add to the
/// drawn mouth. At `mouth_open = 1.0` the mouth ellipse gains this
/// many pixels of total height (i.e. this many pixels of `radius_y`
/// growth mirrored across the center line).
///
/// Chosen to land roughly in line with the Surprised weight-100
/// ellipse (40 px tall) so a loud-speech mouth reads as "open" without
/// towering over the eyes.
const MOUTH_OPEN_MAX_HEIGHT_PX: f32 = 40.0;

/// Draw the mouth. Decision tree:
///
/// 1. `curve != 0`: stroked parabolic arc. `curve > 0` (Happy) smiles,
///    `curve < 0` (Sad) frowns. `Mouth::weight` and `mouth_open` are
///    ignored — arcs stay as the v0.1.0 smile/frown look. (Follow-up
///    can composite an audio-driven open ellipse behind the arc.)
/// 2. Else: filled ellipse whose height is the maximum of the
///    weight-derived height (emotion's static open-mouth — Surprised
///    uses this) and the `mouth_open`-derived audio height. When both
///    are zero, falls back to a horizontal resting line (v0.1.0
///    neutral mouth).
fn draw_mouth<D>(
    mouth: &Mouth,
    curve: i8,
    mouth_color: Rgb565,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    if curve != 0 {
        // Smile/frown sag goes the opposite way from eyes: `curve > 0`
        // (smile) dips the middle below the corners.
        let sag = i32::from(curve) * i32::from(mouth.radius_y) / 100;
        return draw_parabolic_arc(
            mouth.center.x,
            mouth.center.y,
            mouth.radius_x,
            sag,
            stroke(mouth_color, LINE_WIDTH),
            target,
        );
    }

    let weight_height = scaled_height(mouth.radius_y, mouth.weight);
    let audio_height = audio_open_height(mouth.mouth_open);
    let height = weight_height.max(audio_height);
    if height == 0 {
        return draw_horizontal_line(
            mouth.center.x,
            mouth.center.y,
            mouth.radius_x,
            stroke(mouth_color, LINE_WIDTH),
            target,
        );
    }

    let width = mouth.radius_x.saturating_mul(2);
    let half_w = i32::from(mouth.radius_x);
    let half_h = i32::from(height / 2);
    let top_left = EgPoint::new(mouth.center.x - half_w, mouth.center.y - half_h);
    let size = Size::new(u32::from(width), u32::from(height));

    Ellipse::new(top_left, size)
        .into_styled(fill(mouth_color))
        .draw(target)
}

/// Map `Mouth::mouth_open` (`0.0..=1.0`) to an ellipse height in pixels.
///
/// Non-finite values, values below 0, and values above 1 clamp to
/// `[0.0, 1.0]` before scaling. Returns 0 when `mouth_open` is at or
/// below zero, so a fresh avatar (audio silent) renders the same
/// horizontal line as before this feature landed.
fn audio_open_height(mouth_open: f32) -> u16 {
    let clamped = if mouth_open.is_nan() || mouth_open <= 0.0 {
        0.0
    } else if mouth_open >= 1.0 {
        1.0
    } else {
        mouth_open
    };
    let pixels = clamped * MOUTH_OPEN_MAX_HEIGHT_PX;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixels is clamped to [0, MOUTH_OPEN_MAX_HEIGHT_PX]; fits in u16"
    )]
    let rounded = pixels as u16;
    rounded
}

/// Draw a cheek circle below `eye` with color blended between the
/// palette background and the palette cheek colour by `blush`
/// (0..=255). The blend lives in the palette's colour space rather
/// than always-against-white so a `Dark` palette's cheek reads
/// correctly against the black background.
fn draw_cheek<D>(
    eye: &Eye,
    blush: u8,
    eye_scale: u8,
    palette: PaletteColors,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let scaled_ry = scale_radius(eye.radius_y, eye_scale);
    let radius_signed = i32::from(scaled_ry);
    let cheek_top = eye.center.y + radius_signed + CHEEK_VERTICAL_GAP;
    #[allow(clippy::cast_possible_wrap)]
    let half = (CHEEK_DIAMETER / 2) as i32;
    let top_left = EgPoint::new(eye.center.x - half, cheek_top);
    Circle::new(top_left, CHEEK_DIAMETER)
        .into_styled(fill(blend_blush(blush, palette.background, palette.cheek)))
        .draw(target)
}

/// Linearly blend between `from` and `to` colours by `blush` (0 =
/// pure `from`, 255 = pure `to`). Stays in Rgb565 channel space
/// (5/6/5 bits) so the result is directly renderable.
fn blend_blush(blush: u8, from: Rgb565, to: Rgb565) -> Rgb565 {
    let t = u32::from(blush);
    let lerp = |from_ch: u32, to_ch: u32| -> u8 {
        let delta = from_ch.abs_diff(to_ch);
        let shift = delta * t / 255;
        #[allow(clippy::cast_possible_truncation)]
        let shifted = shift as u8;
        #[allow(clippy::cast_possible_truncation)]
        let base = from_ch as u8;
        if to_ch >= from_ch {
            base.saturating_add(shifted)
        } else {
            base.saturating_sub(shifted)
        }
    };
    Rgb565::new(
        lerp(u32::from(from.r()), u32::from(to.r())),
        lerp(u32::from(from.g()), u32::from(to.g())),
        lerp(u32::from(from.b()), u32::from(to.b())),
    )
}

/// Sample a parabolic arc into a stack-allocated 17-point polyline and
/// draw it with `style`.
///
/// `sag` is the vertical offset of the arc's midpoint relative to the
/// baseline at `cy`, positive = middle below baseline, negative = above.
/// `half_w` is the arc's half-width in pixels.
fn draw_parabolic_arc<D>(
    cx: i32,
    cy: i32,
    half_w: u16,
    sag: i32,
    style: PrimitiveStyle<Rgb565>,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // +1 so the array holds both endpoints. Fixed-size so it lives on
    // the stack in a `no_std` build with no allocator.
    const N: usize = ARC_SEGMENTS as usize + 1;
    let mut points: [EgPoint; N] = [EgPoint::zero(); N];
    let half_w_i = i32::from(half_w);
    // Denominator for the (1 - u²) term. Precomputed once so the inner
    // loop is three multiplies and three divides.
    let n_sq = ARC_SEGMENTS * ARC_SEGMENTS;

    for (i, slot) in points.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let i_i = i as i32;
        // u_scaled spans -ARC_SEGMENTS..=+ARC_SEGMENTS, so (1 - u²)
        // normalized by n_sq runs 0 → 1 → 0 across the arc.
        let u_scaled = 2 * i_i - ARC_SEGMENTS;
        let x = cx + u_scaled * half_w_i / ARC_SEGMENTS;
        let bulge_num = n_sq - u_scaled * u_scaled;
        let y = cy + sag * bulge_num / n_sq;
        *slot = EgPoint::new(x, y);
    }

    Polyline::new(&points).into_styled(style).draw(target)
}

/// Shared primitive: a horizontal line centered on `(cx, cy)` with half-width
/// `half_w`, styled with `style`. Used for closed eyes and the resting mouth
/// to avoid drawing a zero-height degenerate ellipse.
fn draw_horizontal_line<D>(
    cx: i32,
    cy: i32,
    half_w: u16,
    style: PrimitiveStyle<Rgb565>,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let hw = i32::from(half_w);
    Line::new(EgPoint::new(cx - hw, cy), EgPoint::new(cx + hw, cy))
        .into_styled(style)
        .draw(target)
}

/// Multiply a half-axis by a 0..=100 weight, clamped. Performed in `u32` to
/// avoid `u16` overflow on intermediate products.
fn scaled_height(radius_y: u16, weight: u8) -> u16 {
    let full = u32::from(radius_y.saturating_mul(2));
    let scaled = (full * u32::from(weight) / 100).min(full);
    u16::try_from(scaled).unwrap_or(u16::MAX)
}

/// Scale a radius by `eye_scale` where 128 = baseline. `u16` output is
/// clamped defensively so a pathological scale can't produce something
/// wider than the framebuffer.
fn scale_radius(radius: u16, scale: u8) -> u16 {
    // Intermediate math in u32; `radius` is at most ~160 and `scale` is
    // at most 255, so the product is well under u32::MAX.
    let scaled = u32::from(radius) * u32::from(scale) / u32::from(SCALE_DEFAULT);
    u16::try_from(scaled).unwrap_or(u16::MAX)
}

/// Convenience: solid-fill style in `color`.
const fn fill(color: Rgb565) -> PrimitiveStyle<Rgb565> {
    PrimitiveStyleBuilder::new().fill_color(color).build()
}

/// Convenience: stroke-only style in `color` at `width` pixels.
const fn stroke(color: Rgb565, width: u32) -> PrimitiveStyle<Rgb565> {
    PrimitiveStyleBuilder::new()
        .stroke_color(color)
        .stroke_width(width)
        .build()
}

/// Battery indicator top-left anchor.
const BATTERY_X: i32 = 8;
/// Battery indicator top anchor.
const BATTERY_Y: i32 = 8;
/// Battery indicator outer width (excludes the terminal nub).
const BATTERY_BODY_W: u32 = 28;
/// Battery indicator outer height.
const BATTERY_BODY_H: u32 = 12;
/// Stroke width of the battery outline.
const BATTERY_STROKE: u32 = 2;
/// Padding from the outline to each segment fill, in pixels per side.
const BATTERY_INNER_PAD: i32 = 2;
/// Width of the terminal nub on the right of the body.
const BATTERY_NUB_W: u32 = 3;
/// Height of the terminal nub.
const BATTERY_NUB_H: u32 = 6;
/// Battery outline / segment colour — black so it reads against the
/// (typically white) palette background. The corner anchor sits above
/// the eye area so the outline never overlaps the rendered face at
/// any geometry preset.
const BATTERY_COLOR: Rgb565 = Rgb565::BLACK;
/// Critical-bucket fill — red so 0..=9 % reads as urgent regardless of
/// charging state.
const BATTERY_CRITICAL_COLOR: Rgb565 = Rgb565::new(31, 8, 4);
/// Charging-bolt overlay colour — same saturated green used elsewhere
/// for "all good" signals.
const BATTERY_CHARGING_COLOR: Rgb565 = Rgb565::new(4, 50, 8);

/// Draw the battery indicator in the upper-left corner.
///
/// Outline + terminal nub render in [`BATTERY_COLOR`]. Inside the body
/// the bucket selects 0..=4 filled segments (Critical = no segments
/// plus a red body fill; Full = four segments). The charging flag
/// renders a thin green vertical bolt centred over the segments — a
/// minimal, integer-coordinate stand-in for the lightning glyph that
/// stays legible at this scale.
fn draw_battery<D>(overlay: BatteryOverlay, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let body_top_left = EgPoint::new(BATTERY_X, BATTERY_Y);
    let body_size = Size::new(BATTERY_BODY_W, BATTERY_BODY_H);

    if matches!(overlay.bucket, BatteryBucket::Critical) {
        Rectangle::new(body_top_left, body_size)
            .into_styled(fill(BATTERY_CRITICAL_COLOR))
            .draw(target)?;
    }
    Rectangle::new(body_top_left, body_size)
        .into_styled(stroke(BATTERY_COLOR, BATTERY_STROKE))
        .draw(target)?;

    #[allow(
        clippy::cast_possible_wrap,
        reason = "constants are well under i32::MAX"
    )]
    let nub_x = BATTERY_X + BATTERY_BODY_W as i32;
    #[allow(
        clippy::cast_possible_wrap,
        reason = "constants are well under i32::MAX"
    )]
    let nub_y = BATTERY_Y + (BATTERY_BODY_H as i32 - BATTERY_NUB_H as i32) / 2;
    Rectangle::new(
        EgPoint::new(nub_x, nub_y),
        Size::new(BATTERY_NUB_W, BATTERY_NUB_H),
    )
    .into_styled(fill(BATTERY_COLOR))
    .draw(target)?;

    let segments = segment_count(overlay.bucket);
    if segments > 0 {
        let inner_top = BATTERY_Y + BATTERY_INNER_PAD;
        let inner_left = BATTERY_X + BATTERY_INNER_PAD;
        #[allow(clippy::cast_possible_wrap, reason = "constants well under i32::MAX")]
        let inner_w = BATTERY_BODY_W as i32 - 2 * BATTERY_INNER_PAD;
        #[allow(clippy::cast_possible_wrap, reason = "constants well under i32::MAX")]
        let inner_h_i32 = BATTERY_BODY_H as i32 - 2 * BATTERY_INNER_PAD;
        let segment_gap: i32 = 1;
        let total_gap = segment_gap * 3;
        let segment_w = ((inner_w - total_gap) / 4).max(1);
        #[allow(
            clippy::cast_sign_loss,
            reason = "segment_w guarded with max(1); inner_h_i32 is positive by construction"
        )]
        let seg_size = Size::new(segment_w as u32, inner_h_i32.max(1) as u32);
        for i in 0..segments {
            let x = inner_left + i32::from(i) * (segment_w + segment_gap);
            Rectangle::new(EgPoint::new(x, inner_top), seg_size)
                .into_styled(fill(BATTERY_COLOR))
                .draw(target)?;
        }
    }

    if overlay.charging {
        #[allow(
            clippy::cast_possible_wrap,
            reason = "constants are well under i32::MAX"
        )]
        let mid_x = BATTERY_X + BATTERY_BODY_W as i32 / 2;
        let top_y = BATTERY_Y + BATTERY_INNER_PAD;
        #[allow(
            clippy::cast_possible_wrap,
            reason = "constants are well under i32::MAX"
        )]
        let bottom_y = BATTERY_Y + BATTERY_BODY_H as i32 - BATTERY_INNER_PAD;
        Line::new(EgPoint::new(mid_x, top_y), EgPoint::new(mid_x, bottom_y))
            .into_styled(stroke(BATTERY_CHARGING_COLOR, 2))
            .draw(target)?;
    }
    Ok(())
}

/// How many filled segments to draw for each bucket. Critical renders
/// zero segments (the body fill conveys the urgency); Full renders all
/// four.
const fn segment_count(bucket: BatteryBucket) -> u8 {
    match bucket {
        BatteryBucket::Critical => 0,
        BatteryBucket::Low => 1,
        BatteryBucket::Medium => 2,
        BatteryBucket::High => 3,
        BatteryBucket::Full => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_height_bounds() {
        assert_eq!(scaled_height(25, 0), 0);
        assert_eq!(scaled_height(25, 100), 50);
        assert_eq!(scaled_height(25, 50), 25);
        // weight > 100 (out-of-contract) must not exceed the full span.
        assert!(scaled_height(25, 200) <= 50);
    }

    #[test]
    fn scale_radius_passes_default_unchanged() {
        assert_eq!(scale_radius(25, SCALE_DEFAULT), 25);
    }

    #[test]
    fn scale_radius_scales_up_and_down() {
        assert_eq!(scale_radius(25, 64), 12);
        assert_eq!(scale_radius(25, 255), 49);
    }

    #[test]
    fn battery_segment_count_matches_bucket() {
        assert_eq!(segment_count(BatteryBucket::Critical), 0);
        assert_eq!(segment_count(BatteryBucket::Low), 1);
        assert_eq!(segment_count(BatteryBucket::Medium), 2);
        assert_eq!(segment_count(BatteryBucket::High), 3);
        assert_eq!(segment_count(BatteryBucket::Full), 4);
    }

    #[test]
    fn blend_blush_endpoints_match_palette() {
        // Default palette: blush 0 → background (white),
        // blush 255 → cheek (pink).
        let default_palette = crate::palette::Palette::Default.colors();
        let at_zero = blend_blush(0, default_palette.background, default_palette.cheek);
        assert_eq!(
            at_zero, default_palette.background,
            "blush=0 stays at background"
        );
        let at_max = blend_blush(255, default_palette.background, default_palette.cheek);
        assert_eq!(at_max, default_palette.cheek, "blush=255 stays at cheek");
    }
}
