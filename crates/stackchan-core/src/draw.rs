//! Render an [`Avatar`] onto any [`DrawTarget`] whose color is [`Rgb565`].
//!
//! The draw code is `no_std`, non-allocating, and hardware-agnostic. The
//! same `Avatar::draw` call runs against `mipidsi::Display` on the CoreS3
//! and against a `Vec<Rgb565>`-backed framebuffer in `stackchan-sim`'s
//! snapshot tests.
//!
//! ## Palette
//!
//! - Background: `Rgb565::WHITE`.
//! - Eyes: `Rgb565::BLACK`, either filled ellipses (when
//!   [`Avatar::eye_curve`] is 0) or a stroked polyline arc (otherwise).
//! - Mouth: pink (`MOUTH_COLOR`), either the v0.1.0 line/ellipse (when
//!   [`Avatar::mouth_curve`] is 0) or a stroked polyline curve.
//! - Cheeks: a weight-blended white→pink circle below each eye when
//!   [`Avatar::cheek_blush`] is non-zero.
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
    pixelcolor::{Rgb565, RgbColor},
    primitives::{
        Circle, Ellipse, Line, Polyline, Primitive, PrimitiveStyle, PrimitiveStyleBuilder,
    },
};

use crate::avatar::{Avatar, Eye, EyePhase, Mouth, SCALE_DEFAULT};

/// Pink mouth/cheek color — `#F58080` quantized into Rgb565's (5,6,5)-bit channels.
const MOUTH_COLOR: Rgb565 = Rgb565::new(30, 32, 16);

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

impl Avatar {
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
        target.clear(Rgb565::WHITE)?;
        // Cheeks first: the eye sits on top of the cheek circle when the
        // two overlap at high `eye_scale` + `cheek_blush`.
        if self.cheek_blush > 0 {
            draw_cheek(&self.left_eye, self.cheek_blush, self.eye_scale, target)?;
            draw_cheek(&self.right_eye, self.cheek_blush, self.eye_scale, target)?;
        }
        draw_eye(&self.left_eye, self.eye_curve, self.eye_scale, target)?;
        draw_eye(&self.right_eye, self.eye_curve, self.eye_scale, target)?;
        draw_mouth(&self.mouth, self.mouth_curve, target)?;
        Ok(())
    }
}

/// Draw one eye. Decision tree:
///
/// 1. Closed phase, or `weight == 0`: horizontal closed-eye line (unchanged
///    v0.1.0 behavior; curves don't apply when the lid is shut).
/// 2. `curve == 0`: filled ellipse, with radii scaled by `eye_scale`.
/// 3. Otherwise: a stroked parabolic arc. `curve > 0` (Happy) arches
///    upward, `curve < 0` (Sad) dips downward.
#[allow(clippy::similar_names)] // `scaled_rx` / `scaled_ry` is the intended x/y pair.
fn draw_eye<D>(eye: &Eye, curve: i8, scale: u8, target: &mut D) -> Result<(), D::Error>
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
            stroke(Rgb565::BLACK, LINE_WIDTH),
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
            .into_styled(fill(Rgb565::BLACK))
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
        stroke(Rgb565::BLACK, EYE_ARC_WIDTH),
        target,
    )
}

/// Draw the mouth. Decision tree:
///
/// 1. `curve != 0`: stroked parabolic arc. `curve > 0` (Happy) smiles,
///    `curve < 0` (Sad) frowns. `Mouth::weight` is ignored.
/// 2. Else `weight == 0`: horizontal resting line (v0.1.0 neutral mouth).
/// 3. Else: filled ellipse whose height scales with `weight` (v0.1.0
///    open-mouth behavior — Surprised uses this path).
fn draw_mouth<D>(mouth: &Mouth, curve: i8, target: &mut D) -> Result<(), D::Error>
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
            stroke(MOUTH_COLOR, LINE_WIDTH),
            target,
        );
    }

    let height = scaled_height(mouth.radius_y, mouth.weight);
    if height == 0 {
        return draw_horizontal_line(
            mouth.center.x,
            mouth.center.y,
            mouth.radius_x,
            stroke(MOUTH_COLOR, LINE_WIDTH),
            target,
        );
    }

    let width = mouth.radius_x.saturating_mul(2);
    let half_w = i32::from(mouth.radius_x);
    let half_h = i32::from(height / 2);
    let top_left = EgPoint::new(mouth.center.x - half_w, mouth.center.y - half_h);
    let size = Size::new(u32::from(width), u32::from(height));

    Ellipse::new(top_left, size)
        .into_styled(fill(MOUTH_COLOR))
        .draw(target)
}

/// Draw a cheek circle below `eye` with color blended between white and
/// `MOUTH_COLOR` by `blush` (0..=255).
fn draw_cheek<D>(eye: &Eye, blush: u8, eye_scale: u8, target: &mut D) -> Result<(), D::Error>
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
        .into_styled(fill(blend_blush(blush)))
        .draw(target)
}

/// Linearly blend between white and `MOUTH_COLOR` by `blush` (0 = white,
/// 255 = full pink). Stays in Rgb565 channel space (5/6/5 bits) to keep
/// the result directly renderable.
fn blend_blush(blush: u8) -> Rgb565 {
    let t = u32::from(blush);
    let lerp = |from: u32, to: u32| -> u8 {
        let delta = from.abs_diff(to);
        let shift = delta * t / 255;
        #[allow(clippy::cast_possible_truncation)]
        let shifted = shift as u8;
        #[allow(clippy::cast_possible_truncation)]
        let base = from as u8;
        if to >= from {
            base.saturating_add(shifted)
        } else {
            base.saturating_sub(shifted)
        }
    };
    Rgb565::new(
        lerp(31, u32::from(MOUTH_COLOR.r())),
        lerp(63, u32::from(MOUTH_COLOR.g())),
        lerp(31, u32::from(MOUTH_COLOR.b())),
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
    fn blend_blush_endpoints_match_palette() {
        let at_zero = blend_blush(0);
        assert_eq!(at_zero, Rgb565::WHITE, "blush=0 is pure white");
        let at_max = blend_blush(255);
        assert_eq!(at_max, MOUTH_COLOR, "blush=255 matches palette pink");
    }
}
