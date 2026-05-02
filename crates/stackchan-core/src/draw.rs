//! Render a [`Face`] onto any [`DrawTarget`] whose color is [`Rgb565`].
//!
//! The draw code is `no_std`, non-allocating, and hardware-agnostic. The
//! same `Face::draw` call runs against `mipidsi::Display` on the CoreS3
//! and against a `Vec<Rgb565>`-backed framebuffer in `stackchan-sim`'s
//! snapshot tests.
//!
//! ## Palette
//!
//! Glossy-emoji refresh: warm off-white background with subtle corner
//! vignettes, layered eyes that read as reflective spheres, feathered
//! cheek blush, and a mouth gloss highlight.
//!
//! - Background: warm off-white (`BG_COLOR`), with four soft corner
//!   vignette arcs (`VIGNETTE_COLOR`) anchoring the frame.
//! - Eyes (open + [`Style::eye_curve`](crate::face::Style::eye_curve)
//!   is 0): three concentric layers — a dark-grey outer ring
//!   (`EYE_OUTER_COLOR`), a pure-black inner core, and a small white
//!   catch-light highlight (`HIGHLIGHT_COLOR`) at upper-left, implying
//!   a single light source above and to the left of the avatar.
//! - Eyes (curved or closed): single stroked polyline arc / horizontal
//!   line, drawn in the dark-grey outer color so they sit consistently
//!   against the layered open-eye look.
//! - Mouth: pink (`MOUTH_COLOR`), either v0.1.0 line/ellipse (when
//!   [`Style::mouth_curve`](crate::face::Style::mouth_curve) is 0) or
//!   a stroked polyline curve. The ellipse variant gains a small white
//!   gloss highlight that mirrors the eye catch-light.
//! - Cheeks: three concentric circles at decreasing blush intensity
//!   (faint outer → medium middle → saturated core), drawn when
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
    pixelcolor::{Rgb565, RgbColor},
    primitives::{
        Circle, Ellipse, Line, Polyline, Primitive, PrimitiveStyle, PrimitiveStyleBuilder,
    },
};

use crate::face::{Eye, EyePhase, Face, Mouth, SCALE_DEFAULT};

/// Warm off-white background — `#FAF7F2` quantized into Rgb565's
/// (5,6,5)-bit channels. Replaces the pure-white v0.1.0 background.
const BG_COLOR: Rgb565 = Rgb565::new(30, 61, 29);

/// Subtle corner-vignette color — a touch warmer + darker than
/// `BG_COLOR`. Drawn as four stroked arcs centered just outside each
/// screen corner, so only the inner edge of each arc shows as a soft
/// curved shadow at the corners.
const VIGNETTE_COLOR: Rgb565 = Rgb565::new(28, 55, 26);

/// Outer eye ring color — `#2A2A2A` dark grey. Sits one band outside
/// the inner black core so the eye reads as a glossy sphere with a
/// soft penumbra rather than a flat disc.
const EYE_OUTER_COLOR: Rgb565 = Rgb565::new(5, 10, 5);

/// Catch-light / gloss highlight color (eyes + mouth).
const HIGHLIGHT_COLOR: Rgb565 = Rgb565::WHITE;

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

/// Cheek outer-ring diameter (the faint outermost circle of the
/// feathered three-ring blush).
const CHEEK_DIAMETER: u32 = 22;

/// Vertical gap between the bottom of an eye and the top of its cheek.
const CHEEK_VERTICAL_GAP: i32 = 6;

/// How many pixels the inner black core of a layered eye is inset from
/// the outer dark-grey ring on each side. Small enough that the ring
/// reads as a thin highlight, big enough that it survives `eye_scale`
/// modulation down to the smallest expected size.
const EYE_RING_INSET: u16 = 3;

/// Half-width of the eye catch-light ellipse. Sized so the highlight
/// reads as a single specular dot against the inner black core without
/// dominating the eye.
const EYE_HIGHLIGHT_HALF_W: i32 = 4;
/// Half-height of the eye catch-light ellipse, paired with
/// [`EYE_HIGHLIGHT_HALF_W`].
const EYE_HIGHLIGHT_HALF_H: i32 = 3;

/// Minimum drawn eye height (in scaled pixels) at which the catch-light
/// is rendered. Below this, the eye is mid-blink or extremely
/// scaled-down and the highlight would clip into the eye boundary.
const EYE_HIGHLIGHT_MIN_HEIGHT: u16 = 14;

/// Half-width of the mouth gloss highlight ellipse. Mirrors the eye
/// catch-light at smaller scale.
const MOUTH_HIGHLIGHT_HALF_W: i32 = 5;
/// Half-height of the mouth gloss highlight ellipse, paired with
/// [`MOUTH_HIGHLIGHT_HALF_W`].
const MOUTH_HIGHLIGHT_HALF_H: i32 = 2;

/// Minimum drawn mouth height (in scaled pixels) at which the gloss
/// highlight is rendered. Below this the mouth ellipse is too thin to
/// host the highlight without clipping.
const MOUTH_HIGHLIGHT_MIN_HEIGHT: u16 = 12;

/// Diameter of each corner vignette arc. Centered at the screen
/// corners, so the visible quarter-arc reaches roughly this/2 pixels
/// into the framebuffer.
const VIGNETTE_DIAMETER: u32 = 160;

/// Stroke width of each corner vignette arc.
const VIGNETTE_WIDTH: u32 = 6;

/// Framebuffer width in pixels. The geometry in [`Face::default`] is
/// tuned for this canvas, and the corner vignette is positioned
/// against this constant.
const FB_WIDTH: i32 = 320;
/// Framebuffer height in pixels, paired with [`FB_WIDTH`].
const FB_HEIGHT: i32 = 240;

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
        target.clear(BG_COLOR)?;
        draw_corner_vignette(target)?;
        // Cheeks first: the eye sits on top of the cheek circle when the
        // two overlap at high `eye_scale` + `cheek_blush`.
        if self.style.cheek_blush > 0 {
            draw_cheek(
                &self.left_eye,
                self.style.cheek_blush,
                self.style.eye_scale,
                target,
            )?;
            draw_cheek(
                &self.right_eye,
                self.style.cheek_blush,
                self.style.eye_scale,
                target,
            )?;
        }
        draw_eye(
            &self.left_eye,
            self.style.eye_curve,
            self.style.eye_scale,
            target,
        )?;
        draw_eye(
            &self.right_eye,
            self.style.eye_curve,
            self.style.eye_scale,
            target,
        )?;
        draw_mouth(&self.mouth, self.style.mouth_curve, target)?;
        Ok(())
    }
}

/// Draw one eye. Decision tree:
///
/// 1. Closed phase, or `weight == 0`: horizontal closed-eye line in the
///    dark-grey outer color.
/// 2. `curve == 0`: layered filled ellipse — outer dark-grey ring +
///    inner black core + small white catch-light at upper-left.
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
            stroke(EYE_OUTER_COLOR, LINE_WIDTH),
            target,
        );
    }

    if curve == 0 {
        return draw_layered_eye(eye.center.x, eye.center.y, scaled_rx, height, target);
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
        stroke(EYE_OUTER_COLOR, EYE_ARC_WIDTH),
        target,
    )
}

/// Render the open + uncurved eye as three concentric layers plus a
/// catch-light: a dark-grey outer ring, a pure-black inner core, and a
/// small white highlight at the upper-left of the core.
///
/// `width` and `height` are the *outer* drawn dimensions of the eye in
/// pixels. The inner core is inset by [`EYE_RING_INSET`] on each side;
/// the catch-light only renders when the drawn eye is taller than
/// [`EYE_HIGHLIGHT_MIN_HEIGHT`] so it doesn't clip during a partial
/// blink.
fn draw_layered_eye<D>(
    cx: i32,
    cy: i32,
    scaled_rx: u16,
    height: u16,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let width = scaled_rx.saturating_mul(2);
    let half_w = i32::from(width / 2);
    let half_h = i32::from(height / 2);
    let top_left = EgPoint::new(cx - half_w, cy - half_h);
    let size = Size::new(u32::from(width), u32::from(height));
    Ellipse::new(top_left, size)
        .into_styled(fill(EYE_OUTER_COLOR))
        .draw(target)?;

    let inset = EYE_RING_INSET.saturating_mul(2);
    let inner_w = width.saturating_sub(inset);
    let inner_h = height.saturating_sub(inset);
    if inner_w > 0 && inner_h > 0 {
        let inner_half_w = i32::from(inner_w / 2);
        let inner_half_h = i32::from(inner_h / 2);
        let inner_top_left = EgPoint::new(cx - inner_half_w, cy - inner_half_h);
        let inner_size = Size::new(u32::from(inner_w), u32::from(inner_h));
        Ellipse::new(inner_top_left, inner_size)
            .into_styled(fill(Rgb565::BLACK))
            .draw(target)?;
    }

    if height >= EYE_HIGHLIGHT_MIN_HEIGHT {
        // Single light source from upper-left: nudge the highlight
        // toward (-rx*0.4, -ry*0.5) of the eye center.
        let hx = cx - i32::from(scaled_rx) * 2 / 5;
        let hy = cy - i32::from(height) / 4;
        draw_filled_ellipse(
            hx,
            hy,
            EYE_HIGHLIGHT_HALF_W,
            EYE_HIGHLIGHT_HALF_H,
            HIGHLIGHT_COLOR,
            target,
        )?;
    }

    Ok(())
}

/// Draw the four corner vignette arcs. Each is a stroked circle
/// centered at a screen corner; only the inner quarter-arc lands
/// inside the framebuffer, reading as a soft curved shadow that
/// frames the face.
fn draw_corner_vignette<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[allow(clippy::cast_possible_wrap)]
    let half = (VIGNETTE_DIAMETER / 2) as i32;
    let style = stroke(VIGNETTE_COLOR, VIGNETTE_WIDTH);
    let corners = [
        EgPoint::new(-half, -half),                      // top-left
        EgPoint::new(FB_WIDTH - half, -half),            // top-right
        EgPoint::new(-half, FB_HEIGHT - half),           // bottom-left
        EgPoint::new(FB_WIDTH - half, FB_HEIGHT - half), // bottom-right
    ];
    for top_left in corners {
        Circle::new(top_left, VIGNETTE_DIAMETER)
            .into_styled(style)
            .draw(target)?;
    }
    Ok(())
}

/// Draw a filled ellipse centered at `(cx, cy)` with the given
/// half-width and half-height. Convenience wrapper around the
/// `embedded-graphics` `Ellipse` constructor's top-left + size form.
fn draw_filled_ellipse<D>(
    cx: i32,
    cy: i32,
    half_w: i32,
    half_h: i32,
    color: Rgb565,
    target: &mut D,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    #[allow(clippy::cast_sign_loss)]
    let w = (half_w.max(0) * 2) as u32;
    #[allow(clippy::cast_sign_loss)]
    let h = (half_h.max(0) * 2) as u32;
    if w == 0 || h == 0 {
        return Ok(());
    }
    Ellipse::new(EgPoint::new(cx - half_w, cy - half_h), Size::new(w, h))
        .into_styled(fill(color))
        .draw(target)
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

    let weight_height = scaled_height(mouth.radius_y, mouth.weight);
    let audio_height = audio_open_height(mouth.mouth_open);
    let height = weight_height.max(audio_height);
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
        .draw(target)?;

    // Gloss highlight near the top of the mouth, mirroring the eye
    // catch-light. Skipped on very thin mouths where the highlight
    // would clip out of the ellipse.
    if height >= MOUTH_HIGHLIGHT_MIN_HEIGHT {
        let hy = mouth.center.y - i32::from(height) / 4;
        draw_filled_ellipse(
            mouth.center.x,
            hy,
            MOUTH_HIGHLIGHT_HALF_W,
            MOUTH_HIGHLIGHT_HALF_H,
            HIGHLIGHT_COLOR,
            target,
        )?;
    }

    Ok(())
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

/// Diameter of the middle cheek ring, faded from outer to inner.
const CHEEK_MIDDLE_DIAMETER: u32 = 14;
/// Diameter of the saturated inner cheek core.
const CHEEK_INNER_DIAMETER: u32 = 8;

/// Draw three concentric cheek circles below `eye` for a feathered
/// blush. Outer ring is faintest (≈30 % of input blush); middle is
/// midway (≈65 %); inner core uses the full input blush. Drawn
/// outer-first so each smaller circle paints over the previous ring's
/// center, producing a soft airbrushed gradient on Rgb565 without a
/// real alpha channel.
fn draw_cheek<D>(eye: &Eye, blush: u8, eye_scale: u8, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let scaled_ry = scale_radius(eye.radius_y, eye_scale);
    let radius_signed = i32::from(scaled_ry);
    #[allow(clippy::cast_possible_wrap)]
    let outer_half = (CHEEK_DIAMETER / 2) as i32;
    let cy = eye.center.y + radius_signed + CHEEK_VERTICAL_GAP + outer_half;
    let cx = eye.center.x;

    let rings: [(u32, u8); 3] = [
        (CHEEK_DIAMETER, blush_scaled(blush, 30)),
        (CHEEK_MIDDLE_DIAMETER, blush_scaled(blush, 65)),
        (CHEEK_INNER_DIAMETER, blush),
    ];
    for (diam, b) in rings {
        if b == 0 {
            continue;
        }
        #[allow(clippy::cast_possible_wrap)]
        let half = (diam / 2) as i32;
        let top_left = EgPoint::new(cx - half, cy - half);
        Circle::new(top_left, diam)
            .into_styled(fill(blend_blush(b)))
            .draw(target)?;
    }
    Ok(())
}

/// Scale `blush` by `percent` and clamp to a `u8`. Used by the
/// feathered cheek to derive outer / middle ring intensities from the
/// input blush.
fn blush_scaled(blush: u8, percent: u32) -> u8 {
    let scaled = u32::from(blush) * percent / 100;
    u8::try_from(scaled.min(u32::from(u8::MAX))).unwrap_or(u8::MAX)
}

/// Linearly blend between [`BG_COLOR`] and [`MOUTH_COLOR`] by `blush`
/// (0 = background, 255 = full pink). Uses the background color (not
/// pure white) as the no-blush endpoint so the outermost feathered
/// cheek ring fades seamlessly into the off-white canvas. Stays in
/// Rgb565 channel space (5/6/5 bits) to keep the result directly
/// renderable.
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
        lerp(u32::from(BG_COLOR.r()), u32::from(MOUTH_COLOR.r())),
        lerp(u32::from(BG_COLOR.g()), u32::from(MOUTH_COLOR.g())),
        lerp(u32::from(BG_COLOR.b()), u32::from(MOUTH_COLOR.b())),
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
        assert_eq!(at_zero, BG_COLOR, "blush=0 fades into the background");
        let at_max = blend_blush(255);
        assert_eq!(at_max, MOUTH_COLOR, "blush=255 matches palette pink");
    }

    #[test]
    fn blush_scaled_clamps_and_scales() {
        assert_eq!(blush_scaled(0, 100), 0);
        assert_eq!(blush_scaled(100, 100), 100);
        assert_eq!(blush_scaled(200, 50), 100);
        assert_eq!(blush_scaled(200, 30), 60);
        assert_eq!(blush_scaled(255, 200), u8::MAX);
    }
}
