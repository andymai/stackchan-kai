//! RGB565 → per-block luma reduction.
//!
//! Splits a frame into a `blocks_x` × `blocks_y` grid of equal-sized
//! cells and writes the mean luma per cell into a caller-provided
//! output array. Pure arithmetic, no allocation, no borrows of the
//! frame outlive the call.
//!
//! ## Coordinate convention
//!
//! Pixels are laid out row-major: byte index for pixel `(x, y)` is
//! `(y * width + x) * BYTES_PER_PIXEL`. Each pixel is two bytes,
//! big-endian (the `LCD_CAM` peripheral's default emit order — high
//! byte first).
//!
//! ## Luma approximation
//!
//! `y ≈ (R8 + 2·G8 + B8) >> 2` — adds-and-shifts only, ~5 ops per
//! pixel. Visually close enough to true Rec. 601 for motion-detection
//! purposes (the tracker compares deltas, not absolute values, so the
//! extra precision of full-weight Rec. 601 multiplies isn't worth the
//! cycles).

/// RGB565 = 2 bytes per pixel.
pub const BYTES_PER_PIXEL: usize = 2;

/// Maximum supported horizontal grid extent.
///
/// Bigger = more arithmetic per frame and more `prev_grid` state.
/// 16 columns over a 320 px frame is one cell every 20 px — finer
/// than necessary for centroid localisation given typical servo and
/// motion lag.
pub const MAX_BLOCKS_X: usize = 16;

/// Maximum supported vertical grid extent.
pub const MAX_BLOCKS_Y: usize = 16;

/// Maximum total cells. The tracker reserves a fixed-size buffer of
/// this length for the previous-frame grid.
pub const MAX_BLOCKS: usize = MAX_BLOCKS_X * MAX_BLOCKS_Y;

/// Compute the per-block mean luma for one frame.
///
/// `frame` must be at least `width * height * BYTES_PER_PIXEL` bytes
/// of big-endian RGB565. `blocks_x` × `blocks_y` cells (≤ [`MAX_BLOCKS_X`] /
/// [`MAX_BLOCKS_Y`]) of width `floor(width / blocks_x)` and height
/// `floor(height / blocks_y)` are scanned. `subsample_step` skips
/// pixels inside each block: `1` reads every pixel, `2` every other
/// pixel in both axes.
///
/// `out[0 .. blocks_x * blocks_y]` is filled with mean luma values in
/// `[0, 255]`; entries past that range are untouched.
///
/// Pixels outside the integer-divisible region (when `width` or
/// `height` doesn't divide evenly) are silently dropped — the
/// algorithm doesn't need exact area parity since fired-cell counting
/// is on a per-block basis.
#[allow(
    clippy::cast_possible_truncation,
    reason = "block_w/block_h are bounded by frame dims (≤ u16::MAX); the \
              `as u16` / `as u8` casts on luma sums are explicitly clamped \
              by /(pixel count) so the value fits."
)]
pub fn fill_block_luma(
    frame: &[u8],
    width: u16,
    height: u16,
    blocks_x: u16,
    blocks_y: u16,
    subsample_step: u8,
    out: &mut [u8],
) {
    if blocks_x == 0 || blocks_y == 0 {
        return;
    }
    let bx = usize::from(blocks_x);
    let by = usize::from(blocks_y);
    let w = usize::from(width);
    let h = usize::from(height);
    let block_w = w / bx;
    let block_h = h / by;
    if block_w == 0 || block_h == 0 {
        return;
    }
    let step = usize::from(subsample_step.max(1));

    for iy in 0..by {
        for ix in 0..bx {
            let x0 = ix * block_w;
            let y0 = iy * block_h;
            let mut sum: u32 = 0;
            let mut count: u32 = 0;
            let mut y = y0;
            while y < y0 + block_h {
                let row_off = y * w * BYTES_PER_PIXEL;
                let mut x = x0;
                while x < x0 + block_w {
                    let off = row_off + x * BYTES_PER_PIXEL;
                    // Bounds check folded in: `off + 1 < frame.len()`.
                    if off + 1 < frame.len() {
                        let hi = frame[off];
                        let lo = frame[off + 1];
                        let pixel = (u16::from(hi) << 8) | u16::from(lo);
                        sum += u32::from(luma_from_rgb565(pixel));
                        count += 1;
                    }
                    x += step;
                }
                y += step;
            }
            let cell_idx = iy * bx + ix;
            if cell_idx < out.len() {
                out[cell_idx] = sum.checked_div(count).unwrap_or(0) as u8;
            }
        }
    }
}

/// Approximate luma from one RGB565 pixel.
///
/// `y ≈ (R8 + 2·G8 + B8) / 4` — fast, surprisingly faithful for motion
/// purposes. R5 / B5 expand to 8-bit by replicating the top 3 bits;
/// G6 expands by replicating the top 2 bits.
#[inline]
#[must_use]
pub const fn luma_from_rgb565(pixel: u16) -> u8 {
    let r5 = ((pixel >> 11) & 0x1F) as u32;
    let g6 = ((pixel >> 5) & 0x3F) as u32;
    let b5 = (pixel & 0x1F) as u32;
    let r8 = (r5 << 3) | (r5 >> 2);
    let g8 = (g6 << 2) | (g6 >> 4);
    let b8 = (b5 << 3) | (b5 >> 2);
    let y = (r8 + 2 * g8 + b8) >> 2;
    // y is bounded by (255 + 2*255 + 255)/4 = 255.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "y is computed with weights summing to 4; max value 255 fits in u8"
    )]
    let y_u8 = y as u8;
    y_u8
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    reason = "tests assert exact integer outputs"
)]
mod tests {
    use super::*;

    #[test]
    fn luma_extremes() {
        // Pure black RGB565 = 0x0000 → luma 0.
        assert_eq!(luma_from_rgb565(0x0000), 0);
        // Pure white RGB565 = 0xFFFF → luma 255.
        assert_eq!(luma_from_rgb565(0xFFFF), 255);
    }

    #[test]
    fn block_luma_uniform_frame() {
        // 16×16 RGB565, all pixels 0xFFFF → every block should be 255.
        let mut frame = alloc::vec::Vec::with_capacity(16 * 16 * 2);
        for _ in 0..(16 * 16) {
            frame.push(0xFF);
            frame.push(0xFF);
        }
        let mut out = [0u8; MAX_BLOCKS];
        fill_block_luma(&frame, 16, 16, 4, 4, 1, &mut out);
        for cell in &out[..16] {
            assert_eq!(*cell, 255);
        }
    }

    #[test]
    fn block_luma_split_frame() {
        // Left half of a 16×16 frame is white, right half black.
        // With a 2×1 grid we expect block 0 = ~255, block 1 = 0.
        let mut frame = alloc::vec::Vec::with_capacity(16 * 16 * 2);
        for y in 0..16 {
            for x in 0..16 {
                let (hi, lo) = if x < 8 {
                    (0xFFu8, 0xFFu8)
                } else {
                    (0x00, 0x00)
                };
                let _ = y; // silence unused warning if optimised away
                frame.push(hi);
                frame.push(lo);
            }
        }
        let mut out = [0u8; MAX_BLOCKS];
        fill_block_luma(&frame, 16, 16, 2, 1, 1, &mut out);
        assert_eq!(out[0], 255);
        assert_eq!(out[1], 0);
    }

    #[test]
    fn fill_skips_when_grid_zero() {
        let frame = [0u8; 32];
        let mut out = [42u8; MAX_BLOCKS];
        fill_block_luma(&frame, 4, 4, 0, 4, 1, &mut out);
        // Untouched (still 42) because blocks_x == 0 short-circuits.
        assert_eq!(out[0], 42);
    }

    extern crate alloc;
}
