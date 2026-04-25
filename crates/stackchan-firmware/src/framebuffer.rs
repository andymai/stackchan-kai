//! In-memory `Rgb565` framebuffer backed by a PSRAM allocation.
//!
//! Every frame the render task draws the composed avatar into this buffer
//! and then blits the whole buffer to the LCD in a single `fill_contiguous`
//! (which mipidsi overrides as one `CASET`/`RASET`/`RAMWR` + bulk SPI
//! write). The buffer eliminates the `target.clear(WHITE)` flicker that
//! direct-draw produces at 30 FPS.
//!
//! 320 × 240 × 2 bytes = 150 KiB — too large for internal SRAM, so the
//! Vec is allocated via `esp_alloc::psram_allocator!` registered in
//! `main`.

use alloc::vec;
use alloc::vec::Vec;

use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::{Rgb565, RgbColor},
    primitives::Rectangle,
};

/// LCD canvas width in pixels.
pub const WIDTH: u32 = 320;
/// LCD canvas height in pixels.
pub const HEIGHT: u32 = 240;

/// `Rgb565` framebuffer with the same row-major layout the ILI9342C expects.
///
/// Implements [`DrawTarget`] with `Infallible` errors; out-of-bounds pixel
/// writes are silently dropped, matching embedded-graphics' `OriginDimensions`
/// clipping contract.
pub struct Framebuffer {
    /// Row-major pixel buffer of length `WIDTH * HEIGHT`, allocated in PSRAM.
    pixels: Vec<Rgb565>,
}

impl Framebuffer {
    /// Allocate a new framebuffer, initialized to white. Expects an
    /// `esp_alloc` PSRAM region to be registered; otherwise the 150 KiB
    /// `vec![]` overflows the 72 KiB internal SRAM heap.
    #[must_use]
    pub fn new() -> Self {
        // `as usize` is lossless: WIDTH*HEIGHT = 76_800 fits in u32 and usize.
        let len = (WIDTH as usize).saturating_mul(HEIGHT as usize);
        Self {
            pixels: vec![Rgb565::WHITE; len],
        }
    }

    /// Borrow the underlying pixel slice (row-major, `WIDTH * HEIGHT` long).
    /// Consumed by the blit path to feed one iterator to `fill_contiguous`.
    #[must_use]
    pub fn as_slice(&self) -> &[Rgb565] {
        &self.pixels
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let Ok(x) = u32::try_from(point.x) else {
                continue;
            };
            let Ok(y) = u32::try_from(point.y) else {
                continue;
            };
            if x >= WIDTH || y >= HEIGHT {
                continue;
            }
            let Ok(idx) = usize::try_from(y.saturating_mul(WIDTH).saturating_add(x)) else {
                continue;
            };
            if let Some(cell) = self.pixels.get_mut(idx) {
                *cell = color;
            }
        }
        Ok(())
    }

    /// Fast path for the `target.clear(WHITE)` at the top of every
    /// `Face::draw` call — avoids the per-pixel `draw_iter` loop by
    /// memset-ing the whole buffer. At `76_800` pixels this saves ~200 µs
    /// per frame vs. the default impl.
    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.pixels.fill(color);
        Ok(())
    }

    /// Fast path for rectangular fills (used by `Face::draw`'s eye/mouth
    /// ellipse bounding operations). Clips to the canvas, then memsets each
    /// row in the overlap — much cheaper than the default `fill_contiguous`
    /// chain, which would iterate `76_800` pixels regardless of rect size.
    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let canvas = Rectangle::new(
            embedded_graphics::geometry::Point::zero(),
            Size::new(WIDTH, HEIGHT),
        );
        let clipped = area.intersection(&canvas);
        if clipped.size.width == 0 || clipped.size.height == 0 {
            return Ok(());
        }
        let Ok(x) = u32::try_from(clipped.top_left.x) else {
            return Ok(());
        };
        let Ok(y) = u32::try_from(clipped.top_left.y) else {
            return Ok(());
        };
        for row in 0..clipped.size.height {
            let row_y = y.saturating_add(row);
            let Ok(start_idx) = usize::try_from(row_y.saturating_mul(WIDTH).saturating_add(x))
            else {
                continue;
            };
            let Ok(span) = usize::try_from(clipped.size.width) else {
                continue;
            };
            let end_idx = start_idx.saturating_add(span);
            if let Some(row_slice) = self.pixels.get_mut(start_idx..end_idx) {
                row_slice.fill(color);
            }
        }
        Ok(())
    }
}
