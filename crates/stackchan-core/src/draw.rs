//! Render an [`Avatar`] onto any [`DrawTarget`] whose color is [`Rgb565`].
//!
//! The draw code is `no_std`, non-allocating, and hardware-agnostic. The
//! same `Avatar::draw` call runs against `mipidsi::Display` on the CoreS3
//! and against a `Vec<Rgb565>`-backed framebuffer in `stackchan-sim`'s
//! snapshot tests.
//!
//! Palette: white background, black eyes (or a thin black line when
//! [`EyePhase::Closed`] or the weight has collapsed to zero height), and a
//! pink mouth (solid line at rest, filled ellipse when open).

use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::{Point as EgPoint, Size},
    pixelcolor::{Rgb565, RgbColor},
    primitives::{Ellipse, Line, Primitive, PrimitiveStyle, PrimitiveStyleBuilder},
};

use crate::avatar::{Avatar, Eye, EyePhase, Mouth};

/// Pink mouth color — `#F58080` quantized into Rgb565's (5,6,5)-bit channels.
const MOUTH_COLOR: Rgb565 = Rgb565::new(30, 32, 16);

/// Stroke width used for both the closed-eye hyphen and the resting mouth line.
const LINE_WIDTH: u32 = 3;

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
        draw_eye(&self.left_eye, target)?;
        draw_eye(&self.right_eye, target)?;
        draw_mouth(&self.mouth, target)?;
        Ok(())
    }
}

/// Draw one eye: a filled black ellipse when open, or a thin horizontal
/// line when closed (or fully squished via `weight == 0`).
fn draw_eye<D>(eye: &Eye, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let height = eye.height();
    if matches!(eye.phase, EyePhase::Closed) || height == 0 {
        return draw_horizontal_line(
            eye.center.x,
            eye.center.y,
            eye.radius_x,
            stroke(Rgb565::BLACK, LINE_WIDTH),
            target,
        );
    }

    let width = eye.width();
    // i32::from(u16) is lossless; half-axes are bounded by the 320x240 canvas.
    let half_w = i32::from(width / 2);
    let half_h = i32::from(height / 2);
    let top_left = EgPoint::new(eye.center.x - half_w, eye.center.y - half_h);
    let size = Size::new(u32::from(width), u32::from(height));

    Ellipse::new(top_left, size)
        .into_styled(fill(Rgb565::BLACK))
        .draw(target)
}

/// Draw the mouth: a pink horizontal line at rest (`weight == 0`), or a
/// filled pink ellipse whose height scales with `weight`.
fn draw_mouth<D>(mouth: &Mouth, target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
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

/// Shared primitive: a horizontal line centered on `(cx, cy)` with half-width
/// `half_w`, styled with `style`. Used for both closed eyes and the resting
/// mouth to avoid drawing a zero-height degenerate ellipse.
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
}
