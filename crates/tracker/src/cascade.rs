//! Pure-Rust Viola–Jones face cascade scoring.
//!
//! `no_std`, integer arithmetic where possible, allocation-free. The
//! scorer works against a caller-supplied luma plane (one byte per
//! pixel, row-major) and a [`Cascade`] of stump-based weak classifiers
//! arranged into stages. Cascade weights are produced offline by the
//! `xtask-cascade-convert` workspace binary which parses `OpenCV`'s
//! reference `haarcascade_frontalface_default.xml`.
//!
//! ## Pipeline
//!
//! 1. Caller derives a luma plane for the ROI to scan
//!    (the firmware extracts it from the RGB565 DMA buffer; tests pass
//!    a synthesized one).
//! 2. [`IntegralView`] computes the sum and sum-of-squares integral
//!    images for the ROI on the fly.
//! 3. For each `(x, y, scale)` window position [`Cascade::evaluate`]
//!    runs the stages, rejecting on the first stage whose accumulated
//!    score falls below the stage threshold.
//! 4. [`Cascade::scan`] sweeps a multi-scale grid across the ROI and
//!    returns the best-scoring detection (or `None`).
//!
//! ## Variance normalisation
//!
//! `OpenCV`'s stump evaluation compares the raw weighted-rect sum to
//! `threshold × variance_norm × window_area`, where
//! `variance_norm = sqrt(sumSqs · area − sum²)`. The standard deviation
//! cancels per-window contrast scaling. We follow the same convention so
//! the cascade weights produced by the XML converter need no rescaling.
//!
//! ## Coordinate convention
//!
//! Feature rectangles in [`Feature`] are stored in the cascade's base
//! window space (typically 24 × 24 for the default frontal cascade).
//! The scorer multiplies by the current `scale` (Q16.16 fixed-point) at
//! evaluation time so the cascade data structure is scale-independent.

use crate::luma::BYTES_PER_PIXEL;

/// One weighted rectangle inside a cascade [`Feature`].
///
/// Coordinates are in the cascade's base window — the scorer multiplies
/// by the current scale before reading the integral image. The base
/// window is typically 24 × 24, so `u8` is more than enough range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Top-left x in base-window units.
    pub x: u8,
    /// Top-left y in base-window units.
    pub y: u8,
    /// Width in base-window units.
    pub w: u8,
    /// Height in base-window units.
    pub h: u8,
    /// Per-rectangle weight, as published in the XML
    /// (`-1.0`, `+1.0`, `+2.0`, …). `OpenCV` stores these as floats; we
    /// keep them as `i8` since every published weight in the frontal
    /// cascades is a small integer. The XML converter will reject any
    /// fractional weight.
    pub weight: i8,
}

/// Maximum rectangles per [`Feature`]. `OpenCV`'s published frontal
/// cascades use 2 or 3; we allocate room for 3.
pub const MAX_RECTS_PER_FEATURE: usize = 3;

/// One Haar feature: 2 or 3 weighted rectangles whose differential sum
/// is the discriminator. The actual count lives in [`Feature::rect_count`]
/// — slots beyond that are unused.
#[derive(Debug, Clone, Copy)]
pub struct Feature {
    /// Up to [`MAX_RECTS_PER_FEATURE`] rectangles. Slots `>= rect_count`
    /// are unused; their contents are unspecified.
    pub rects: [Rect; MAX_RECTS_PER_FEATURE],
    /// Number of valid entries in [`Self::rects`] (`2` or `3`).
    pub rect_count: u8,
}

/// One stump-based weak classifier ("tree").
///
/// All published frontal cascades from `OpenCV` are stump-based — a
/// single threshold split with two leaf values. The cascade format here
/// intentionally does NOT support deeper trees; the XML converter
/// rejects them.
#[derive(Debug, Clone, Copy)]
pub struct Stump {
    /// Discriminating feature.
    pub feature: Feature,
    /// Comparison threshold in window-area-normalised units. The scorer
    /// multiplies by `variance_norm × window_area_inv` before
    /// comparing. Stored as `f32` since the XML threshold is fractional
    /// at full precision.
    pub threshold: f32,
    /// Score added when the feature value is below the threshold.
    pub left_val: f32,
    /// Score added when the feature value is at or above the threshold.
    pub right_val: f32,
}

/// One stage of the cascade. A window must clear every stage's
/// threshold to count as a face.
#[derive(Debug, Clone, Copy)]
pub struct Stage {
    /// Slice of stumps belonging to this stage. `'static` because the
    /// cascade data is a `const` baked into flash.
    pub stumps: &'static [Stump],
    /// Cumulative-score threshold for this stage. The scorer rejects
    /// the window the first time the running sum falls below this
    /// value at the end of a stage.
    pub threshold: f32,
}

/// A full cascade: base window dimensions plus an ordered stage list.
#[derive(Debug, Clone, Copy)]
pub struct Cascade {
    /// Base window width in pixels (typically 24).
    pub window_w: u8,
    /// Base window height in pixels (typically 24).
    pub window_h: u8,
    /// Ordered stages. The scorer evaluates them in slice order.
    pub stages: &'static [Stage],
}

impl Cascade {
    /// Iterate the cascade base size as `usize` for window-area math.
    #[must_use]
    pub const fn window_area(&self) -> u32 {
        (self.window_w as u32) * (self.window_h as u32)
    }
}

/// Outcome of [`Cascade::scan`]: the best-scoring window inside an ROI.
///
/// Coordinates are in the same pixel space the caller passed to
/// [`IntegralView::from_luma`] — i.e. ROI-local, top-left origin. The
/// caller is responsible for translating back to frame-local
/// coordinates if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    /// ROI-local x of the detected window's top-left corner.
    pub x: u16,
    /// ROI-local y of the detected window's top-left corner.
    pub y: u16,
    /// Detection window width in ROI pixels (= `window_w × scale`).
    pub w: u16,
    /// Detection window height in ROI pixels.
    pub h: u16,
    /// Scale factor at which the cascade fired, as a Q16.16 multiplier
    /// of the base window. `1.0 ↦ 65_536`. Useful for downstream
    /// post-processing (NMS, scale-aware merging).
    pub scale_q16: u32,
    /// Stage at which the window passed (`stages.len()` if every stage
    /// accepted; lower numbers indicate early-exit on a stump-only
    /// scoring pass — currently always equals `stages.len()`).
    pub stages_passed: u16,
}

/// Sum and sum-of-squares integral images over an arbitrary 8-bit luma
/// plane.
///
/// Memory layout: each integral image is `(w + 1) × (h + 1)` cells with
/// a zero-padded first row and column. Cell `(x, y)` holds the sum of
/// every pixel strictly above and to the left of pixel `(x, y)` in the
/// source plane. This lets `rect_sum(x, y, w, h)` work as
/// `I(x+w, y+h) − I(x, y+h) − I(x+w, y) + I(x, y)`.
///
/// The view borrows two caller-allocated buffers so the scanner can
/// reuse them across calls without `alloc`.
pub struct IntegralView<'a> {
    /// Sum integral image. Length `(w + 1) × (h + 1)`.
    sum: &'a [u32],
    /// Sum-of-squares integral image. Length `(w + 1) × (h + 1)`.
    sum_sq: &'a [u64],
    /// Source plane width in pixels.
    width: u16,
    /// Source plane height in pixels.
    height: u16,
}

impl<'a> IntegralView<'a> {
    /// Compute the integral images of an 8-bit luma plane into the
    /// caller-supplied buffers.
    ///
    /// Both `sum_buf` and `sum_sq_buf` must have at least
    /// `(width + 1) × (height + 1)` entries. Excess entries are left
    /// untouched.
    ///
    /// Returns `None` if either buffer is too small.
    #[must_use]
    pub fn from_luma(
        luma: &[u8],
        width: u16,
        height: u16,
        sum_buf: &'a mut [u32],
        sum_sq_buf: &'a mut [u64],
    ) -> Option<Self> {
        let w = usize::from(width);
        let h = usize::from(height);
        let stride = w + 1;
        let needed = stride * (h + 1);
        if sum_buf.len() < needed || sum_sq_buf.len() < needed || luma.len() < w * h {
            return None;
        }
        // Zero the first row + first column. We only zero the cells we
        // touch; the rest of the buffer is left as-is.
        for x in 0..stride {
            sum_buf[x] = 0;
            sum_sq_buf[x] = 0;
        }
        for y in 1..=h {
            sum_buf[y * stride] = 0;
            sum_sq_buf[y * stride] = 0;
        }
        for y in 0..h {
            let mut row_sum: u32 = 0;
            let mut row_sum_sq: u64 = 0;
            for x in 0..w {
                let pix = u32::from(luma[y * w + x]);
                row_sum += pix;
                row_sum_sq += u64::from(pix) * u64::from(pix);
                let ii_idx = (y + 1) * stride + (x + 1);
                let above_idx = y * stride + (x + 1);
                sum_buf[ii_idx] = sum_buf[above_idx] + row_sum;
                sum_sq_buf[ii_idx] = sum_sq_buf[above_idx] + row_sum_sq;
            }
        }
        Some(Self {
            sum: sum_buf,
            sum_sq: sum_sq_buf,
            width,
            height,
        })
    }

    /// Source plane width.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Source plane height.
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// Sum of pixel values in the rectangle `[x, x+w) × [y, y+h)`.
    ///
    /// `x + w` and `y + h` must be `≤` the plane dimensions. Out-of-bounds
    /// rectangles return `0` rather than panic — the scanner guarantees
    /// in-bounds access by construction, this is a defensive fallback.
    #[must_use]
    pub fn rect_sum(&self, rx: u16, ry: u16, rw: u16, rh: u16) -> u32 {
        if u32::from(rx) + u32::from(rw) > u32::from(self.width)
            || u32::from(ry) + u32::from(rh) > u32::from(self.height)
        {
            return 0;
        }
        let stride = usize::from(self.width) + 1;
        let x0 = usize::from(rx);
        let y0 = usize::from(ry);
        let x1 = x0 + usize::from(rw);
        let y1 = y0 + usize::from(rh);
        let top_l = self.sum[y0 * stride + x0];
        let top_r = self.sum[y0 * stride + x1];
        let bot_l = self.sum[y1 * stride + x0];
        let bot_r = self.sum[y1 * stride + x1];
        bot_r + top_l - top_r - bot_l
    }

    /// Sum of squared pixel values in the rectangle `[x, x+w) × [y, y+h)`.
    ///
    /// Same bounds rules as [`Self::rect_sum`].
    #[must_use]
    pub fn rect_sum_sq(&self, rx: u16, ry: u16, rw: u16, rh: u16) -> u64 {
        if u32::from(rx) + u32::from(rw) > u32::from(self.width)
            || u32::from(ry) + u32::from(rh) > u32::from(self.height)
        {
            return 0;
        }
        let stride = usize::from(self.width) + 1;
        let x0 = usize::from(rx);
        let y0 = usize::from(ry);
        let x1 = x0 + usize::from(rw);
        let y1 = y0 + usize::from(rh);
        let top_l = self.sum_sq[y0 * stride + x0];
        let top_r = self.sum_sq[y0 * stride + x1];
        let bot_l = self.sum_sq[y1 * stride + x0];
        let bot_r = self.sum_sq[y1 * stride + x1];
        bot_r + top_l - top_r - bot_l
    }
}

/// Convert an interleaved big-endian RGB565 frame into a luma plane,
/// in place into a caller-provided byte buffer.
///
/// Required output length is `width × height`. Returns the number of
/// pixels written, or `0` if the buffers are too small.
///
/// This is a convenience helper for the firmware's camera task — host
/// tests can build their own luma planes directly without going through
/// RGB565.
pub fn luma_from_rgb565_frame(frame: &[u8], width: u16, height: u16, out: &mut [u8]) -> usize {
    let w = usize::from(width);
    let h = usize::from(height);
    let needed = w * h;
    if out.len() < needed || frame.len() < needed * BYTES_PER_PIXEL {
        return 0;
    }
    for y in 0..h {
        for x in 0..w {
            let off = (y * w + x) * BYTES_PER_PIXEL;
            let pixel = (u16::from(frame[off]) << 8) | u16::from(frame[off + 1]);
            out[y * w + x] = crate::luma::luma_from_rgb565(pixel);
        }
    }
    needed
}

impl Cascade {
    /// Evaluate the cascade at one specific window position + scale.
    ///
    /// Returns `Some(stage_count)` if every stage accepted (the window
    /// is classified as a face). Returns `None` if any stage rejected.
    ///
    /// `scale_q16` is a Q16.16 fixed-point multiplier of the base
    /// window — `65_536` means 1×, `81_920` means 1.25×, etc. Using
    /// fixed-point keeps the inner loop integer-only.
    ///
    /// `wx`, `wy` are the top-left of the candidate window in the
    /// integral image's pixel space.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "All numeric domains are bounded by `MAX_ROI = 96`: \
                  `area ≤ 96² = 9_216`, `win_sum ≤ 255 × 9_216 ≈ 2.4 M`, \
                  `win_sum_sq ≤ 255² × 9_216 ≈ 5.99 × 10⁸`, and \
                  `var_term ≤ win_sum_sq × area ≈ 5.5 × 10¹²` — well \
                  inside `i64` (~9.2 × 10¹⁸). var_term is also \
                  algebraically non-negative (Cauchy–Schwarz), so the \
                  `u64 → i64` cast cannot wrap. f32's 23-bit mantissa \
                  loses ~3 LSBs on the largest var_term values; the \
                  cascade's sub-stddev thresholds tolerate this loss."
    )]
    pub fn evaluate(&self, ii: &IntegralView<'_>, wx: u16, wy: u16, scale_q16: u32) -> Option<u16> {
        // Scaled window dimensions in pixels.
        let win_w = scale_dim(self.window_w, scale_q16);
        let win_h = scale_dim(self.window_h, scale_q16);
        if win_w == 0 || win_h == 0 {
            return None;
        }
        if u32::from(wx) + u32::from(win_w) > u32::from(ii.width())
            || u32::from(wy) + u32::from(win_h) > u32::from(ii.height())
        {
            return None;
        }
        let area = u64::from(win_w) * u64::from(win_h);
        // Variance norm: sqrt(sumSqs * area - sum * sum). Used to
        // contrast-normalise feature thresholds. Clamped at 1 to avoid
        // div-by-zero on flat regions where every stump trivially fires.
        let win_sum = u64::from(ii.rect_sum(wx, wy, win_w, win_h));
        let win_sum_sq = ii.rect_sum_sq(wx, wy, win_w, win_h);
        // Equivalent to `sumSqs · area − sum²` evaluated as `i64`.
        // Casting both sides to i64 first then subtracting keeps the
        // expression unambiguous to the reader; both terms are
        // bounded as documented in the function-level allow above.
        let term_left = (win_sum_sq as i64).saturating_mul(area as i64);
        let term_right = (win_sum as i64).saturating_mul(win_sum as i64);
        let var_term = term_left.saturating_sub(term_right);
        let var_norm = if var_term <= 0 {
            1.0_f32
        } else {
            // sqrt over the f64 promotion keeps precision for QVGA-scale
            // sums (peak ~150 KB pixels × 255 = 39 M ≪ 2^32).
            sqrt_f64(var_term as f64) as f32
        };
        let inv_area = 1.0_f32 / (area as f32);

        for stage in self.stages {
            let mut sum: f32 = 0.0;
            for stump in stage.stumps {
                let feat = feature_value(&stump.feature, ii, wx, wy, scale_q16) as f32;
                // Compare against threshold * varNorm * area; the
                // feature value is a raw weighted pixel-sum so we
                // compensate for window area on both sides.
                let normed = feat * inv_area;
                let cmp = stump.threshold * var_norm * inv_area;
                sum += if normed < cmp {
                    stump.left_val
                } else {
                    stump.right_val
                };
            }
            if sum < stage.threshold {
                return None;
            }
        }
        // Total stage count is at most u16::MAX in practice; the
        // OpenCV frontal default has 25 stages.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "stages.len() ≤ u16::MAX in any plausible cascade"
        )]
        Some(self.stages.len() as u16)
    }

    /// Sweep windows across the integral image and return the best
    /// detection (largest stage-pass count; ties broken by largest
    /// window).
    ///
    /// `step_q16` is the scale-multiplier between successive scan
    /// scales (e.g. `78_643` = 1.2 ×). `min_scale_q16` and
    /// `max_scale_q16` bound the scale range relative to the cascade
    /// base window.
    ///
    /// `pixel_step` is the slide step in pixels between successive
    /// window positions at each scale; `OpenCV` uses
    /// `max(1, scale × 2)`-ish but a constant `2` is fine for QVGA.
    #[must_use]
    pub fn scan(
        &self,
        ii: &IntegralView<'_>,
        min_scale_q16: u32,
        max_scale_q16: u32,
        step_q16: u32,
        pixel_step: u16,
    ) -> Option<Detection> {
        let mut best: Option<Detection> = None;
        let mut scale_q16 = min_scale_q16.max(Q16_ONE);
        let step_q16 = step_q16.max(Q16_ONE + 1);
        let pixel_step = pixel_step.max(1);
        while scale_q16 <= max_scale_q16 {
            let win_w = scale_dim(self.window_w, scale_q16);
            let win_h = scale_dim(self.window_h, scale_q16);
            if win_w == 0 || win_h == 0 || win_w > ii.width() || win_h > ii.height() {
                break;
            }
            let max_x = ii.width() - win_w;
            let max_y = ii.height() - win_h;
            let mut wy: u16 = 0;
            while wy <= max_y {
                let mut wx: u16 = 0;
                while wx <= max_x {
                    if let Some(stages_passed) = self.evaluate(ii, wx, wy, scale_q16) {
                        let det = Detection {
                            x: wx,
                            y: wy,
                            w: win_w,
                            h: win_h,
                            scale_q16,
                            stages_passed,
                        };
                        match best {
                            None => best = Some(det),
                            Some(prev) if better(&det, &prev) => best = Some(det),
                            _ => {}
                        }
                    }
                    wx = wx.saturating_add(pixel_step);
                }
                wy = wy.saturating_add(pixel_step);
            }
            // Multiply scale: scale_q16 = scale_q16 * step_q16 / Q16_ONE.
            let next = (u64::from(scale_q16) * u64::from(step_q16)) / u64::from(Q16_ONE);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "scale_q16 ≤ max_scale_q16 ≤ u32::MAX by construction; \
                          the multiplication-and-divide stays within u32 range \
                          for any plausible cascade scan."
            )]
            let next_u32 = next.min(u64::from(u32::MAX)) as u32;
            if next_u32 <= scale_q16 {
                break;
            }
            scale_q16 = next_u32;
        }
        best
    }
}

/// Q16.16 representation of `1.0`.
pub const Q16_ONE: u32 = 1 << 16;

/// Multiply a base-window dimension by a Q16.16 scale, rounding to
/// the nearest pixel.
const fn scale_dim(base: u8, scale_q16: u32) -> u16 {
    let scaled = (base as u64) * (scale_q16 as u64);
    // Round-to-nearest: add Q16_ONE / 2 before shifting.
    let rounded = (scaled + (Q16_ONE as u64) / 2) >> 16;
    if rounded > u16::MAX as u64 {
        u16::MAX
    } else {
        #[allow(clippy::cast_possible_truncation, reason = "checked above")]
        let r = rounded as u16;
        r
    }
}

/// Compute one feature value: the weighted sum of its rectangles in
/// the integral image, in pixel-sum units.
fn feature_value(feat: &Feature, ii: &IntegralView<'_>, wx: u16, wy: u16, scale_q16: u32) -> i64 {
    let mut acc: i64 = 0;
    let count = (feat.rect_count as usize).min(MAX_RECTS_PER_FEATURE);
    for slot in 0..count {
        let r = &feat.rects[slot];
        let rx = wx + scale_dim(r.x, scale_q16);
        let ry = wy + scale_dim(r.y, scale_q16);
        let rw = scale_dim(r.w, scale_q16);
        let rh = scale_dim(r.h, scale_q16);
        let s = i64::from(ii.rect_sum(rx, ry, rw, rh));
        acc += s * i64::from(r.weight);
    }
    acc
}

/// Tie-breaker for `Cascade::scan`: prefer more stages passed, then
/// larger detection windows (more confident scale).
fn better(candidate: &Detection, current: &Detection) -> bool {
    if candidate.stages_passed != current.stages_passed {
        return candidate.stages_passed > current.stages_passed;
    }
    u32::from(candidate.w) * u32::from(candidate.h) > u32::from(current.w) * u32::from(current.h)
}

/// `f64::sqrt` re-exposed through libm so the crate stays `no_std`.
#[inline]
fn sqrt_f64(x: f64) -> f64 {
    libm::sqrt(x)
}

/// Maximum supported ROI dimension for the on-chip face scanner.
///
/// The firmware extracts an `MAX_ROI × MAX_ROI` luma window around
/// each candidate centroid; values larger than this are clamped to
/// fit the scratch buffers. 96 px is roughly four cascade base windows
/// tall — generous enough that a face occupying ~⅓ of the QVGA frame
/// fits without scaling artefacts.
pub const MAX_ROI: u16 = 96;

/// Default minimum scan scale for [`Cascade::scan_around_centroid`],
/// in Q16.16 fixed-point. `1.0×` of the cascade base window (24 px).
pub const SCAN_MIN_SCALE_Q16: u32 = Q16_ONE;

/// Default maximum scan scale (`2.0×` of the cascade base window).
///
/// On-device profiling on CoreS3 showed the original `3.0×` upper
/// bound was too generous — typical interaction-distance faces fire
/// at `1.0×` / `1.25×` anyway, and the `2.0×` cap roughly halves the
/// per-frame cascade workload.
pub const SCAN_MAX_SCALE_Q16: u32 = Q16_ONE * 2;

/// Default scale-step multiplier between successive scan scales.
///
/// `1.4×` (Q16.16) per step covers the `1.0×`–`2.0×` range in two
/// steps, missing the in-between `1.2×`/`1.5×` sizes — but a face at
/// 30 px is classified by either the 24 px or 34 px window with the
/// cascade design, so missing exact scale ratios costs little real
/// recall. Empirical: dropping from `1.2×` to `1.4×` per step
/// (4 → 2 scales) roughly halved per-frame cascade time on the
/// CoreS3 face-bench run.
pub const SCAN_SCALE_STEP_Q16: u32 = (Q16_ONE * 7) / 5;

/// Default pixel-step between successive scan positions at each
/// scale.
///
/// `2` matches `OpenCV`'s default for the published frontal cascade.
/// On-device profiling on CoreS3 first tried `4` (a 4× position-count
/// savings), but the cascade then missed obvious frontal faces — Haar
/// features depend on the face being centered within ~10–20 % of the
/// 24 px base window, and a 4 px step occasionally lands every
/// candidate window just off-axis. `2` keeps the recall the cascade
/// was tuned for; the speedup comes from `SCAN_MAX_SCALE_Q16` /
/// `SCAN_SCALE_STEP_Q16` plus the camera task's frame-skip
/// (`CASCADE_PERIOD`) instead.
pub const SCAN_PIXEL_STEP: u16 = 2;

/// Heap-friendly scratch buffers for [`Cascade::scan_around_centroid`].
///
/// Holds a luma plane plus the two integral images at the maximum
/// supported ROI size. Total footprint is ~120 KiB — fits comfortably
/// in PSRAM but is too large to land on a stack frame, so callers
/// typically allocate one [`CascadeScratch`] via `Box::leak` and reuse
/// it for every frame.
#[allow(
    clippy::large_stack_arrays,
    reason = "120 KiB inline buffers are the entire point of this type — \
              callers always heap-allocate via `Box::leak` so the array \
              never lands on a stack frame."
)]
pub struct CascadeScratch {
    /// Luma plane for the most recent ROI extracted from a camera
    /// frame. Indexed `y * roi_w + x`; only the first
    /// `roi_w × roi_h` bytes are valid after a scan call.
    pub luma: [u8; (MAX_ROI as usize) * (MAX_ROI as usize)],
    /// Sum integral image. Length `(MAX_ROI + 1)²`.
    pub sum: [u32; ((MAX_ROI as usize) + 1) * ((MAX_ROI as usize) + 1)],
    /// Sum-of-squares integral image. Length `(MAX_ROI + 1)²`.
    pub sum_sq: [u64; ((MAX_ROI as usize) + 1) * ((MAX_ROI as usize) + 1)],
}

impl CascadeScratch {
    /// All-zero scratch. Equivalent to [`Default::default`] but `const`.
    #[must_use]
    #[allow(
        clippy::large_stack_arrays,
        reason = "120 KiB inline buffers are the entire point of this type — \
                  callers always heap-allocate via `Box::leak` so the array \
                  never lands on a stack frame."
    )]
    pub const fn new() -> Self {
        Self {
            luma: [0; (MAX_ROI as usize) * (MAX_ROI as usize)],
            sum: [0; ((MAX_ROI as usize) + 1) * ((MAX_ROI as usize) + 1)],
            sum_sq: [0; ((MAX_ROI as usize) + 1) * ((MAX_ROI as usize) + 1)],
        }
    }
}

impl Default for CascadeScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of [`Cascade::scan_around_centroid`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceDetection {
    /// Normalised centroid of the detected face, frame-coordinates,
    /// `[-1.0, 1.0]` per axis. `(0, 0)` is frame centre.
    pub centroid: (f32, f32),
    /// Detection window in frame-local pixels: `(x, y, w, h)`.
    pub frame_rect: (u16, u16, u16, u16),
    /// Stage at which the cascade accepted (currently always equal to
    /// `cascade.stages.len()`; included so future early-exit changes
    /// don't break the API).
    pub stages_passed: u16,
}

impl Cascade {
    /// Scan a region around a normalised centroid for faces.
    ///
    /// `frame` is a big-endian RGB565 buffer of size
    /// `frame_w × frame_h × 2`. `centroid_norm` is in `[-1, 1]` per
    /// axis (the same convention [`crate::Outcome::centroid`] uses).
    /// `roi_dim` is the ROI side length in pixels — the function
    /// clamps it to [`MAX_ROI`] and to the frame bounds, so callers
    /// can pass a comfortably large value (e.g. 96) without worrying
    /// about edge cases.
    ///
    /// Returns `Some(FaceDetection)` if any window inside the ROI
    /// passed every cascade stage. `None` otherwise.
    #[must_use]
    pub fn scan_around_centroid(
        &self,
        frame: &[u8],
        frame_w: u16,
        frame_h: u16,
        centroid_norm: (f32, f32),
        roi_dim: u16,
        scratch: &mut CascadeScratch,
    ) -> Option<FaceDetection> {
        let (roi_x, roi_y, roi_w, roi_h) =
            roi_around_centroid(frame_w, frame_h, centroid_norm, roi_dim)?;

        // Extract the luma plane for the ROI.
        let stride = usize::from(frame_w);
        let roi_pixels_x = usize::from(roi_w);
        let roi_pixels_y = usize::from(roi_h);
        for py in 0..roi_pixels_y {
            for px in 0..roi_pixels_x {
                let fx = usize::from(roi_x) + px;
                let fy = usize::from(roi_y) + py;
                let off = (fy * stride + fx) * BYTES_PER_PIXEL;
                if off + 1 >= frame.len() {
                    return None;
                }
                let pix = (u16::from(frame[off]) << 8) | u16::from(frame[off + 1]);
                scratch.luma[py * roi_pixels_x + px] = crate::luma::luma_from_rgb565(pix);
            }
        }

        // Build the integral views over the ROI.
        let view = IntegralView::from_luma(
            &scratch.luma[..roi_pixels_x * roi_pixels_y],
            roi_w,
            roi_h,
            &mut scratch.sum,
            &mut scratch.sum_sq,
        )?;

        // Multi-scale scan. Tuning constants (`SCAN_*`) were chosen
        // from on-device profiling on CoreS3: the original `1.0×–3.0×`
        // sweep at `1.2×` step + 2 px stride averaged ~500 ms per
        // ROI, ~14× over the 30 FPS budget. The current defaults are
        // ~6× tighter and bring the worst case down to a usable
        // budget without losing detection on typical interaction
        // distances. See cascade module-level docs for the empirical
        // rationale.
        let det = self.scan(
            &view,
            SCAN_MIN_SCALE_Q16,
            SCAN_MAX_SCALE_Q16,
            SCAN_SCALE_STEP_Q16,
            SCAN_PIXEL_STEP,
        )?;

        // Translate ROI-local detection to frame coordinates and
        // normalise the centroid. `libm::fmaf` keeps the rounding
        // error tighter than `(cx / w) * 2.0 - 1.0`; native `mul_add`
        // isn't available on `no_std` Xtensa.
        let frame_x = roi_x + det.x;
        let frame_y = roi_y + det.y;
        // Centre = top-left + half the window. `libm::fmaf` keeps
        // the rounding error tighter than the explicit form; native
        // `mul_add` isn't available on `no_std` Xtensa.
        let cx = libm::fmaf(f32::from(det.w), 0.5, f32::from(frame_x));
        let cy = libm::fmaf(f32::from(det.h), 0.5, f32::from(frame_y));
        let inv_frame_w = 1.0_f32 / f32::from(frame_w);
        let inv_frame_h = 1.0_f32 / f32::from(frame_h);
        let nx = libm::fmaf(cx * inv_frame_w, 2.0, -1.0);
        let ny = libm::fmaf(cy * inv_frame_h, 2.0, -1.0);
        Some(FaceDetection {
            centroid: (nx, ny),
            frame_rect: (frame_x, frame_y, det.w, det.h),
            stages_passed: det.stages_passed,
        })
    }
}

/// Centre an `roi_dim × roi_dim` window on the normalised centroid
/// inside the frame. Clamps to frame bounds; returns `None` if the
/// resulting ROI is degenerate (zero area).
fn roi_around_centroid(
    frame_w: u16,
    frame_h: u16,
    centroid: (f32, f32),
    roi_dim: u16,
) -> Option<(u16, u16, u16, u16)> {
    if frame_w == 0 || frame_h == 0 {
        return None;
    }
    let dim = roi_dim.min(MAX_ROI).min(frame_w).min(frame_h);
    if dim == 0 {
        return None;
    }
    let (nx, ny) = (centroid.0.clamp(-1.0, 1.0), centroid.1.clamp(-1.0, 1.0));
    let cx = (nx + 1.0) * 0.5 * f32::from(frame_w);
    let cy = (ny + 1.0) * 0.5 * f32::from(frame_h);
    let half = f32::from(dim) * 0.5;
    let x0 = (cx - half).clamp(0.0, f32::from(frame_w - dim));
    let y0 = (cy - half).clamp(0.0, f32::from(frame_h - dim));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "x0/y0 are clamped to [0, frame_dim - dim] (≤ u16::MAX) so \
                  the truncation back to u16 cannot lose information"
    )]
    let (rx, ry) = (x0 as u16, y0 as u16);
    Some((rx, ry, dim, dim))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::large_stack_arrays,
    clippy::cast_possible_truncation,
    clippy::similar_names,
    clippy::excessive_precision,
    reason = "tests assert exact-match outputs against synthesised fixtures, \
              build small integral-image stack buffers (49² × 8 B ≈ 19 KiB) \
              that exceed the default 16 KiB threshold but are fine on the \
              host test runner, and use OpenCV-published thresholds at full \
              precision (rounded by the f32 literal) for spot-checks"
)]
mod tests {
    use super::*;

    fn build_view<'a>(
        luma: &[u8],
        w: u16,
        h: u16,
        sum_buf: &'a mut [u32],
        sum_sq_buf: &'a mut [u64],
    ) -> IntegralView<'a> {
        IntegralView::from_luma(luma, w, h, sum_buf, sum_sq_buf).unwrap()
    }

    #[test]
    fn integral_uniform_plane() {
        // 4×4 plane all 10s. Any 2×2 rect sums to 40.
        let luma = [10u8; 16];
        let mut sb = [0u32; 25];
        let mut sqb = [0u64; 25];
        let v = build_view(&luma, 4, 4, &mut sb, &mut sqb);
        assert_eq!(v.rect_sum(0, 0, 4, 4), 160);
        assert_eq!(v.rect_sum(1, 1, 2, 2), 40);
        assert_eq!(v.rect_sum_sq(0, 0, 4, 4), 1600);
        assert_eq!(v.rect_sum_sq(1, 1, 2, 2), 400);
    }

    #[test]
    fn integral_split_plane() {
        // Left half white (255), right half black (0). 4×2.
        let luma = [255, 255, 0, 0, 255, 255, 0, 0];
        let mut sb = [0u32; 15];
        let mut sqb = [0u64; 15];
        let v = build_view(&luma, 4, 2, &mut sb, &mut sqb);
        assert_eq!(v.rect_sum(0, 0, 2, 2), 255 * 4);
        assert_eq!(v.rect_sum(2, 0, 2, 2), 0);
        assert_eq!(v.rect_sum(0, 0, 4, 2), 255 * 4);
    }

    #[test]
    fn integral_rejects_undersized_buffer() {
        let luma = [0u8; 16];
        let mut sb = [0u32; 4];
        let mut sqb = [0u64; 4];
        assert!(IntegralView::from_luma(&luma, 4, 4, &mut sb, &mut sqb).is_none());
    }

    /// Build a tiny one-stage one-stump cascade that fires on a
    /// dark-over-light edge feature in a 4×4 base window.
    fn tiny_edge_cascade() -> (Cascade, [Stage; 1], [Stump; 1]) {
        // Feature: top half (-1 weight) vs bottom half (+1 weight). Net
        // feature value is `(bottom_sum − top_sum)`, large-positive when
        // the bottom half is materially brighter than the top half.
        //
        // The threshold is in "stddev fractions": comparison evaluates
        // `feature_value < threshold × variance_norm`, so a positive
        // threshold (here `0.1 × stddev`) means flat regions land in
        // `left_val` while strong edges land in `right_val`. This
        // mirrors OpenCV's stump semantics; using `threshold = 0`
        // would incorrectly fire on completely flat windows.
        let feat = Feature {
            rects: [
                Rect {
                    x: 0,
                    y: 0,
                    w: 4,
                    h: 2,
                    weight: -1,
                },
                Rect {
                    x: 0,
                    y: 2,
                    w: 4,
                    h: 2,
                    weight: 1,
                },
                Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                    weight: 0,
                },
            ],
            rect_count: 2,
        };
        let stump = Stump {
            feature: feat,
            threshold: 0.1,
            left_val: -1.0,
            right_val: 1.0,
        };
        let stumps_arr: [Stump; 1] = [stump];
        let stage_arr: [Stage; 1] = [Stage {
            stumps: &[],
            threshold: 0.0,
        }];
        let cascade = Cascade {
            window_w: 4,
            window_h: 4,
            stages: &[],
        };
        (cascade, stage_arr, stumps_arr)
    }

    /// Wire the tiny cascade through `'static` references so we can
    /// actually call `evaluate`. Test-only helper; the real cascade
    /// data is generated as `const` arrays by the xtask converter.
    fn leak_tiny_cascade(stages: [Stage; 1], stumps: [Stump; 1]) -> Cascade {
        // Leak so the slices are 'static for the test's lifetime.
        let stumps_box: alloc::boxed::Box<[Stump; 1]> = alloc::boxed::Box::new(stumps);
        let stumps_static: &'static [Stump] = alloc::boxed::Box::leak(stumps_box);
        let stages_with_stumps: alloc::boxed::Box<[Stage; 1]> = alloc::boxed::Box::new([Stage {
            stumps: stumps_static,
            threshold: stages[0].threshold,
        }]);
        let stages_static: &'static [Stage] = alloc::boxed::Box::leak(stages_with_stumps);
        Cascade {
            window_w: 4,
            window_h: 4,
            stages: stages_static,
        }
    }

    #[test]
    fn cascade_fires_on_bright_bottom() {
        // 4×4 luma plane: top half dark, bottom half bright.
        let luma = [
            10, 10, 10, 10, 10, 10, 10, 10, 200, 200, 200, 200, 200, 200, 200, 200,
        ];
        let mut sb = [0u32; 25];
        let mut sqb = [0u64; 25];
        let v = build_view(&luma, 4, 4, &mut sb, &mut sqb);
        let (_dummy_cascade, stages, stumps) = tiny_edge_cascade();
        let cascade = leak_tiny_cascade(stages, stumps);
        let r = cascade.evaluate(&v, 0, 0, Q16_ONE);
        assert!(r.is_some(), "cascade should fire on bright-bottom pattern");
    }

    #[test]
    fn cascade_rejects_inverted_pattern() {
        // Bright top, dark bottom: stump should land in `left_val`,
        // stage sum is negative, fails threshold.
        let luma = [
            200, 200, 200, 200, 200, 200, 200, 200, 10, 10, 10, 10, 10, 10, 10, 10,
        ];
        let mut sb = [0u32; 25];
        let mut sqb = [0u64; 25];
        let v = build_view(&luma, 4, 4, &mut sb, &mut sqb);
        let (_dummy, stages, stumps) = tiny_edge_cascade();
        let cascade = leak_tiny_cascade(stages, stumps);
        assert!(cascade.evaluate(&v, 0, 0, Q16_ONE).is_none());
    }

    #[test]
    fn scan_finds_window_in_larger_roi() {
        // 8×8 plane, bright lower-left 4×4 quadrant. The cascade is the
        // 4×4 "bright-bottom" stump from above; the windows that fire
        // are the ones whose top half is dark and bottom half overlaps
        // the bright quadrant — namely the (0, 2) and (0, 3) windows.
        //
        // Restricted to scale 1.0 because the synthetic 4×4 base
        // cascade has features (`y=2, h=2`) that don't scale cleanly
        // (`2 × 1.25 → 3` rounds the bottom-half rect outside a 5-row
        // window). Real OpenCV cascades sit on a 24×24 base where every
        // feature rect is an exact divisor, so this only bites tests.
        let mut luma = [10u8; 64];
        for y in 4..8 {
            for x in 0..4 {
                luma[y * 8 + x] = 200;
            }
        }
        let mut sb = [0u32; 81];
        let mut sqb = [0u64; 81];
        let v = build_view(&luma, 8, 8, &mut sb, &mut sqb);
        let (_dummy, stages, stumps) = tiny_edge_cascade();
        let cascade = leak_tiny_cascade(stages, stumps);
        // min == max == Q16_ONE → single-scale sweep.
        let det = cascade.scan(&v, Q16_ONE, Q16_ONE, Q16_ONE * 5 / 4, 1);
        let det = det.expect("expected a detection at the bright-quadrant window");
        assert_eq!(det.x, 0, "detection should hug the bright column");
        // Any window whose bottom half overlaps the bright band fires.
        // Scan order picks the first hit (`wy = 1` here, where rows 3–4
        // straddle the dark/bright boundary). Pure dark (wy ≤ 0) and
        // pure bright (wy ≥ 4) windows correctly do NOT fire.
        assert!(
            det.y >= 1 && det.y <= 3,
            "detection y must straddle the dark/bright boundary, got {}",
            det.y,
        );
    }

    /// Build a two-stage cascade where stage 0 always accepts (single
    /// stump that fires on the test pattern) and stage 1 always
    /// rejects (single stump whose feature can't possibly clear an
    /// impossibly-large stage threshold).
    ///
    /// Used to verify the `evaluate` early-exit path: the second-stage
    /// rejection short-circuits before any further stages would run.
    fn impossible_second_stage_cascade() -> Cascade {
        let stump_pass = Stump {
            feature: Feature {
                rects: [
                    Rect {
                        x: 0,
                        y: 0,
                        w: 4,
                        h: 2,
                        weight: -1,
                    },
                    Rect {
                        x: 0,
                        y: 2,
                        w: 4,
                        h: 2,
                        weight: 1,
                    },
                    Rect {
                        x: 0,
                        y: 0,
                        w: 0,
                        h: 0,
                        weight: 0,
                    },
                ],
                rect_count: 2,
            },
            threshold: 0.1,
            left_val: -1.0,
            right_val: 1.0,
        };
        let stump_reject = Stump {
            feature: Feature {
                rects: [
                    Rect {
                        x: 0,
                        y: 0,
                        w: 1,
                        h: 1,
                        weight: 1,
                    },
                    Rect {
                        x: 0,
                        y: 0,
                        w: 0,
                        h: 0,
                        weight: 0,
                    },
                    Rect {
                        x: 0,
                        y: 0,
                        w: 0,
                        h: 0,
                        weight: 0,
                    },
                ],
                rect_count: 1,
            },
            threshold: 0.0,
            left_val: 0.0,
            right_val: 0.0,
        };
        let stumps_pass: alloc::boxed::Box<[Stump; 1]> = alloc::boxed::Box::new([stump_pass]);
        let stumps_reject: alloc::boxed::Box<[Stump; 1]> = alloc::boxed::Box::new([stump_reject]);
        let stumps_pass_static: &'static [Stump] = alloc::boxed::Box::leak(stumps_pass);
        let stumps_reject_static: &'static [Stump] = alloc::boxed::Box::leak(stumps_reject);
        let stages: alloc::boxed::Box<[Stage; 2]> = alloc::boxed::Box::new([
            Stage {
                stumps: stumps_pass_static,
                threshold: 0.0,
            },
            // No stump can produce a positive score (both leaves are 0),
            // so any positive stage threshold rejects unconditionally.
            Stage {
                stumps: stumps_reject_static,
                threshold: 1.0,
            },
        ]);
        Cascade {
            window_w: 4,
            window_h: 4,
            stages: alloc::boxed::Box::leak(stages),
        }
    }

    #[test]
    fn frontal_cascade_geometry() {
        // Sanity: the loaded cascade has the dimensions and stage count
        // we expect from OpenCV's published frontal default.
        let c = crate::FRONTAL_FACE;
        assert_eq!(c.window_w, 24);
        assert_eq!(c.window_h, 24);
        assert_eq!(c.stages.len(), 25);
        // Spot-check the first stage threshold against the XML.
        let s0 = c.stages[0];
        assert_eq!(s0.stumps.len(), 9);
        assert!((s0.threshold - -5.042_55_f32).abs() < 1e-3);
    }

    #[test]
    fn frontal_cascade_rejects_flat_region() {
        // A 32×32 uniform-grey luma plane has zero variance — the
        // cascade must reject every window without panicking.
        let luma = [128u8; 32 * 32];
        let mut sb = [0u32; 33 * 33];
        let mut sqb = [0u64; 33 * 33];
        let v = build_view(&luma, 32, 32, &mut sb, &mut sqb);
        let det = crate::FRONTAL_FACE.scan(&v, Q16_ONE, Q16_ONE, Q16_ONE * 5 / 4, 4);
        assert!(det.is_none(), "real cascade fired on a flat plane");
    }

    #[test]
    fn frontal_cascade_rejects_random_noise() {
        // Pseudo-random luma plane — no face structure, must not fire.
        // Linear-congruential generator keeps the test deterministic.
        let mut seed: u32 = 0xDEAD_BEEF;
        let mut luma = [0u8; 48 * 48];
        for cell in &mut luma {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *cell = (seed >> 16) as u8;
        }
        let mut sb = [0u32; 49 * 49];
        let mut sqb = [0u64; 49 * 49];
        let v = build_view(&luma, 48, 48, &mut sb, &mut sqb);
        let det = crate::FRONTAL_FACE.scan(&v, Q16_ONE, Q16_ONE * 3 / 2, Q16_ONE * 5 / 4, 4);
        assert!(det.is_none(), "real cascade fired on uniform noise");
    }

    #[test]
    fn cascade_rejects_at_second_stage() {
        let luma = [
            10, 10, 10, 10, 10, 10, 10, 10, 200, 200, 200, 200, 200, 200, 200, 200,
        ];
        let mut sb = [0u32; 25];
        let mut sqb = [0u64; 25];
        let v = build_view(&luma, 4, 4, &mut sb, &mut sqb);
        let cascade = impossible_second_stage_cascade();
        // Stage 0 accepts (bright-bottom edge); stage 1 rejects.
        assert!(cascade.evaluate(&v, 0, 0, Q16_ONE).is_none());
    }

    extern crate alloc;
}
