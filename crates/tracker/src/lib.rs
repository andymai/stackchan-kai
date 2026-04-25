//! # tracker
//!
//! Block-grid motion tracker for the Stack-chan camera. Consumes raw
//! RGB565 camera frames, computes inter-frame motion via per-block luma
//! deltas, and emits a target [`Pose`] for the head servos. Pure
//! algorithm — no I/O, no allocation, host-testable.
//!
//! ## Pipeline
//!
//! 1. Convert each pixel to an 8-bit luma estimate (Rec. 601 weights).
//! 2. Sum luma into a small grid of blocks (configurable, ≤ 16×16).
//! 3. Compare the new grid to the previous frame's grid. A block whose
//!    normalised delta exceeds [`TrackerConfig::block_threshold`] fires.
//! 4. If the count of fired cells is above [`TrackerConfig::min_fired_cells`]
//!    and below [`TrackerConfig::max_fired_fraction`] (the latter
//!    rejects whole-frame events like the lights flipping on), compute
//!    the centroid of the fired cells and translate it to a pan/tilt
//!    delta via the configured camera FOV.
//! 5. Apply a dead zone, proportional gain, and per-step slew limit;
//!    accumulate into a target [`Pose`]; clamp via [`Pose::clamped`].
//! 6. If no motion is seen for [`TrackerConfig::idle_timeout_ms`], slew
//!    the target back toward [`Pose::NEUTRAL`].
//!
//! ## Coordinates
//!
//! Frames are assumed to be column-major-byte-pairs in big-endian
//! RGB565 (the `LCD_CAM` peripheral's default byte order). Pixel `(x, y)`
//! lives at byte offset `(y * width + x) * 2`. The tracker does not
//! validate orientation; mirrored cameras can be corrected via the
//! [`TrackerConfig::flip_x`] / [`TrackerConfig::flip_y`] flags.
//!
//! ## Stability
//!
//! Experimental as of v0.1.0.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

mod luma;

pub use luma::{BYTES_PER_PIXEL, MAX_BLOCKS, MAX_BLOCKS_X, MAX_BLOCKS_Y, fill_block_luma};

use stackchan_core::Pose;

/// Configuration for [`Tracker`].
///
/// All fields are public so callers can tune individual parameters
/// from the workspace [`TrackerConfig::DEFAULT`].
#[derive(Debug, Clone, Copy)]
pub struct TrackerConfig {
    /// Frame width in pixels. Must equal the source DMA buffer width.
    pub frame_width: u16,
    /// Frame height in pixels.
    pub frame_height: u16,
    /// Horizontal blocks in the analysis grid. ≤ [`MAX_BLOCKS_X`].
    pub blocks_x: u16,
    /// Vertical blocks in the analysis grid. ≤ [`MAX_BLOCKS_Y`].
    pub blocks_y: u16,
    /// Pixel skip factor inside a block (1 = read every pixel,
    /// 2 = read every other pixel in both axes). Larger values trade
    /// a small amount of accuracy for ~`step²` less memory bandwidth.
    pub subsample_step: u8,
    /// Camera horizontal field of view in degrees. GC0308 with the
    /// CoreS3 lens is roughly 62°.
    pub fov_h_deg: f32,
    /// Camera vertical field of view in degrees. ~49° on the same lens.
    pub fov_v_deg: f32,
    /// Per-block delta threshold in normalised luma units `[0.0, 1.0]`.
    /// A block with `|curr - prev| / 255` above this fires.
    pub block_threshold: f32,
    /// Minimum fired-cell count required to count as "real" motion.
    /// Rejects single-block sensor noise.
    pub min_fired_cells: u16,
    /// If more than this fraction of cells fire on a single frame the
    /// event is treated as a global lighting change and ignored.
    pub max_fired_fraction: f32,
    /// Proportional gain on centroid-offset error in `[0.0, 1.0]`.
    /// `1.0` jumps the head all the way to centre the target on every
    /// frame; `0.4` damps the loop pleasantly given typical servo lag.
    pub p_gain: f32,
    /// Dead-zone half-width as a fraction of frame size, applied
    /// independently per axis. A normalised centroid offset within
    /// `±dead_zone` produces no motion.
    pub dead_zone: f32,
    /// Maximum pose-delta per [`Tracker::step`] call, per axis, in
    /// degrees. Caps angular velocity to keep the loop stable when the
    /// servos can't keep up with the commanded rate.
    pub max_step_deg: f32,
    /// Idle timeout in milliseconds. After this much "no motion" the
    /// tracker begins slewing back to [`Pose::NEUTRAL`].
    pub idle_timeout_ms: u32,
    /// Per-step slew while idle, in degrees per axis. The default
    /// produces a calm return at 30 FPS.
    pub idle_step_deg: f32,
    /// Mirror the centroid horizontally before mapping to pan. Use when
    /// the camera image is left-right reversed relative to the head's
    /// pan direction (e.g. lens / sensor mounted upside-down).
    pub flip_x: bool,
    /// Mirror vertically. Use when the image is upside-down relative
    /// to the head's tilt direction.
    pub flip_y: bool,
}

impl TrackerConfig {
    /// Defaults tuned for QVGA GC0308 → `SCServo` head on a CoreS3.
    ///
    /// 8×6 grid (40×40 pixel blocks), 5 % per-block luma threshold,
    /// 70 % global-event ceiling, P=0.4, ±10 % dead zone, 4 °/frame
    /// slew, 3 s idle timeout, 1 °/step return-to-centre.
    pub const DEFAULT: Self = Self {
        frame_width: 320,
        frame_height: 240,
        blocks_x: 8,
        blocks_y: 6,
        subsample_step: 2,
        fov_h_deg: 62.0,
        fov_v_deg: 49.0,
        block_threshold: 0.05,
        min_fired_cells: 2,
        max_fired_fraction: 0.70,
        p_gain: 0.4,
        dead_zone: 0.10,
        max_step_deg: 4.0,
        idle_timeout_ms: 3_000,
        idle_step_deg: 1.0,
        flip_x: false,
        flip_y: false,
    };
}

/// Outcome of one [`Tracker::step`] call.
///
/// Returned for diagnostic logging in the bench example and for unit
/// tests. The new target pose is always available as [`Outcome::target`];
/// [`Outcome::motion`] distinguishes the *why*.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    /// Number of grid cells whose per-block delta exceeded the threshold.
    /// Capped at `blocks_x * blocks_y`.
    pub fired_cells: u16,
    /// Normalised centroid in `[-1.0, 1.0]` per axis when motion was
    /// detected, else `None`. `(0, 0)` means the centre of the frame.
    pub centroid: Option<(f32, f32)>,
    /// Classification of this step.
    pub motion: Motion,
    /// New target pose after this step. Always within
    /// [`Pose::clamped`]'s safe range.
    pub target: Pose,
}

/// Why the tracker chose this pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Not enough information to act yet — the tracker is buffering its
    /// first frame. Target pose is unchanged.
    Warmup,
    /// Real motion detected; the target pose was nudged toward it.
    Tracking,
    /// No motion this step but still inside the idle timeout window;
    /// target pose held.
    Holding,
    /// Idle long enough to be slewing back toward [`Pose::NEUTRAL`].
    Returning,
    /// Too many cells fired this frame — likely a global lighting
    /// change. Target pose held.
    GlobalEvent,
}

/// Block-grid motion tracker.
///
/// Holds the previous-frame block grid and the running target pose.
/// One [`Tracker`] per camera; not `Sync` (the firmware uses one task
/// per camera so this is fine — see `examples/tracker_bench.rs`).
pub struct Tracker {
    /// Active configuration. Captured at construction time and not
    /// re-validated on every step.
    config: TrackerConfig,
    /// Previous frame's per-block mean luma in `[0, 255]`. Indexed as
    /// `prev_grid[y * blocks_x + x]`. Length is `blocks_x * blocks_y`.
    prev_grid: [u8; MAX_BLOCKS],
    /// Whether [`Self::prev_grid`] holds a real previous frame.
    /// `false` until the first [`Self::step`].
    have_prev: bool,
    /// Running commanded target pose. Always inside the [`Pose::clamped`]
    /// safe range.
    target_pose: Pose,
    /// Accumulated time since motion was last detected, in
    /// milliseconds. Saturates at `u32::MAX`.
    idle_ms: u32,
}

impl Tracker {
    /// Build a tracker from a config.
    ///
    /// Out-of-range grid sizes are silently clamped to
    /// `[1, MAX_BLOCKS_X]` × `[1, MAX_BLOCKS_Y]`; other values are
    /// trusted. Caller is expected to seed [`TrackerConfig`] from
    /// [`TrackerConfig::DEFAULT`] and tweak the few fields they care
    /// about.
    #[must_use]
    pub const fn new(config: TrackerConfig) -> Self {
        Self {
            config,
            prev_grid: [0; MAX_BLOCKS],
            have_prev: false,
            target_pose: Pose::NEUTRAL,
            idle_ms: 0,
        }
    }

    /// Borrow the active config.
    #[must_use]
    pub const fn config(&self) -> &TrackerConfig {
        &self.config
    }

    /// Most-recently commanded target pose.
    #[must_use]
    pub const fn target_pose(&self) -> Pose {
        self.target_pose
    }

    /// Forget the previous frame and reset the target to `NEUTRAL`.
    /// The next [`Self::step`] will report [`Motion::Warmup`].
    pub const fn reset(&mut self) {
        self.have_prev = false;
        self.target_pose = Pose::NEUTRAL;
        self.idle_ms = 0;
    }

    /// Process one frame.
    ///
    /// `frame` must hold at least
    /// `frame_width * frame_height * BYTES_PER_PIXEL` bytes of big-endian
    /// RGB565. Frames smaller than that produce a warmup result with
    /// the target pose unchanged.
    ///
    /// `dt_ms` is the wall-clock interval since the previous step.
    /// Used for the idle-timeout / return-to-centre logic. Pass the
    /// real delta when available, or a constant nominal frame period
    /// (e.g. 33 ms at 30 FPS) when not.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::similar_names,
        reason = "block-grid arithmetic: cell counts ≤ MAX_BLOCKS (256) so \
                  usize→u32 / u32→f32 casts cannot lose information; the \
                  f32→u16 casts are bounded by inputs that have already been \
                  clamped to [0, 255] / [0, total_cells]; per-axis \
                  `nx`/`ny`/`*_eff` pairs are the algorithm's natural names"
    )]
    pub fn step(&mut self, frame: &[u8], dt_ms: u32) -> Outcome {
        let cfg = &self.config;
        let bx = clamp_blocks(cfg.blocks_x, MAX_BLOCKS_X);
        let by = clamp_blocks(cfg.blocks_y, MAX_BLOCKS_Y);
        let total = usize::from(bx) * usize::from(by);

        let need = usize::from(cfg.frame_width) * usize::from(cfg.frame_height) * BYTES_PER_PIXEL;
        if frame.len() < need {
            return Outcome {
                fired_cells: 0,
                centroid: None,
                motion: Motion::Warmup,
                target: self.target_pose,
            };
        }

        let mut curr_grid = [0u8; MAX_BLOCKS];
        fill_block_luma(
            frame,
            cfg.frame_width,
            cfg.frame_height,
            bx,
            by,
            cfg.subsample_step.max(1),
            &mut curr_grid,
        );

        if !self.have_prev {
            self.prev_grid[..total].copy_from_slice(&curr_grid[..total]);
            self.have_prev = true;
            return Outcome {
                fired_cells: 0,
                centroid: None,
                motion: Motion::Warmup,
                target: self.target_pose,
            };
        }

        let threshold_u = ((cfg.block_threshold.clamp(0.0, 1.0)) * 255.0) as u16;
        let max_fired = ((cfg.max_fired_fraction.clamp(0.0, 1.0)) * total as f32) as u16;
        let mut fired_cells: u16 = 0;
        let mut sum_x: u32 = 0;
        let mut sum_y: u32 = 0;
        for iy in 0..usize::from(by) {
            for ix in 0..usize::from(bx) {
                let idx = iy * usize::from(bx) + ix;
                let curr = u16::from(curr_grid[idx]);
                let prev = u16::from(self.prev_grid[idx]);
                let delta = curr.abs_diff(prev);
                if delta > threshold_u {
                    fired_cells += 1;
                    sum_x += ix as u32;
                    sum_y += iy as u32;
                }
            }
        }
        // Roll the previous grid forward unconditionally — even on a
        // global-event frame we want next frame's delta to be against
        // the new state.
        self.prev_grid[..total].copy_from_slice(&curr_grid[..total]);

        if fired_cells > max_fired {
            // Treat as global event. Hold pose, reset idle timer so we
            // don't immediately start returning to centre on lighting
            // flickers (likely the user just walked into the room).
            self.idle_ms = 0;
            return Outcome {
                fired_cells,
                centroid: None,
                motion: Motion::GlobalEvent,
                target: self.target_pose,
            };
        }

        if fired_cells < cfg.min_fired_cells {
            return self.no_motion_outcome(dt_ms, fired_cells);
        }

        // Centroid in block-index space, then normalised to [-1, 1].
        let (cell_cx, cell_cy) = (
            (sum_x as f32) / f32::from(fired_cells),
            (sum_y as f32) / f32::from(fired_cells),
        );
        // Centre of a block-index axis of length N is at (N-1)/2;
        // dividing by that maps the index-space centroid to [-1, 1].
        let half_x = (f32::from(bx) - 1.0) * 0.5;
        let half_y = (f32::from(by) - 1.0) * 0.5;
        let mut nx = (cell_cx - half_x) / half_x.max(0.5);
        let mut ny = (cell_cy - half_y) / half_y.max(0.5);
        if cfg.flip_x {
            nx = -nx;
        }
        if cfg.flip_y {
            ny = -ny;
        }

        // Apply dead zone per axis.
        let dz = cfg.dead_zone.clamp(0.0, 0.99);
        let nx_eff = if nx.abs() < dz { 0.0 } else { nx };
        let ny_eff = if ny.abs() < dz { 0.0 } else { ny };

        // Map normalised offset to angle delta via half-FOV * P.
        // Sign convention:
        //   nx > 0 → motion is right-of-centre → head should pan right
        //                                        → +pan_deg
        //   ny > 0 → motion is below centre    → head should tilt down
        //                                        → -tilt_deg
        // (Recall: positive tilt = nod up.)
        let pan_delta =
            (nx_eff * cfg.fov_h_deg * 0.5 * cfg.p_gain).clamp(-cfg.max_step_deg, cfg.max_step_deg);
        let tilt_delta =
            (-ny_eff * cfg.fov_v_deg * 0.5 * cfg.p_gain).clamp(-cfg.max_step_deg, cfg.max_step_deg);

        self.target_pose = Pose::new(
            self.target_pose.pan_deg + pan_delta,
            self.target_pose.tilt_deg + tilt_delta,
        )
        .clamped();
        self.idle_ms = 0;

        Outcome {
            fired_cells,
            centroid: Some((nx, ny)),
            motion: Motion::Tracking,
            target: self.target_pose,
        }
    }

    /// Helper: build the outcome for a "no motion this frame" step.
    /// Splits the idle-timeout / return-to-centre logic out of
    /// [`Self::step`] for readability.
    const fn no_motion_outcome(&mut self, dt_ms: u32, fired_cells: u16) -> Outcome {
        self.idle_ms = self.idle_ms.saturating_add(dt_ms);
        if self.idle_ms < self.config.idle_timeout_ms {
            return Outcome {
                fired_cells,
                centroid: None,
                motion: Motion::Holding,
                target: self.target_pose,
            };
        }
        let step = self.config.idle_step_deg.max(0.0);
        self.target_pose = Pose::new(
            slew_toward(self.target_pose.pan_deg, 0.0, step),
            slew_toward(self.target_pose.tilt_deg, 0.0, step),
        )
        .clamped();
        Outcome {
            fired_cells,
            centroid: None,
            motion: Motion::Returning,
            target: self.target_pose,
        }
    }
}

/// Step `value` toward `target` by at most `step` units.
const fn slew_toward(value: f32, target: f32, step: f32) -> f32 {
    let diff = target - value;
    if diff.abs() <= step {
        target
    } else if diff > 0.0 {
        value + step
    } else {
        value - step
    }
}

/// Clamp a configured grid extent into `[1, max]`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "max is one of MAX_BLOCKS_X / MAX_BLOCKS_Y, both ≤ 16, fits trivially in u16"
)]
const fn clamp_blocks(requested: u16, max: usize) -> u16 {
    if requested == 0 {
        1
    } else if (requested as usize) > max {
        max as u16
    } else {
        requested
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests assert exact-match outputs of our own arithmetic; \
              unwrap and panic-on-mismatch are fine on synthesised fixtures"
)]
mod tests {
    use super::*;

    /// Build a 320×240 RGB565 frame filled with a single luma value.
    fn flat_frame(luma: u8) -> alloc::vec::Vec<u8> {
        // RGB565 luma ≈ G channel; pick a green value matching `luma`.
        // For a uniform frame we just need every pixel identical, so
        // any encoding is fine — set R=G=B from `luma`.
        let r5 = u16::from(luma >> 3) & 0x1F;
        let g6 = u16::from(luma >> 2) & 0x3F;
        let b5 = u16::from(luma >> 3) & 0x1F;
        let pixel = (r5 << 11) | (g6 << 5) | b5;
        let hi = (pixel >> 8) as u8;
        let lo = (pixel & 0xFF) as u8;
        let mut v = alloc::vec::Vec::with_capacity(320 * 240 * 2);
        for _ in 0..(320 * 240) {
            v.push(hi);
            v.push(lo);
        }
        v
    }

    /// Paint a rectangular bright patch onto an existing frame.
    fn paint_patch(frame: &mut [u8], x0: usize, y0: usize, w: usize, h: usize, luma: u8) {
        let r5 = u16::from(luma >> 3) & 0x1F;
        let g6 = u16::from(luma >> 2) & 0x3F;
        let b5 = u16::from(luma >> 3) & 0x1F;
        let pixel = (r5 << 11) | (g6 << 5) | b5;
        let hi = (pixel >> 8) as u8;
        let lo = (pixel & 0xFF) as u8;
        for y in y0..(y0 + h) {
            for x in x0..(x0 + w) {
                let idx = (y * 320 + x) * 2;
                frame[idx] = hi;
                frame[idx + 1] = lo;
            }
        }
    }

    extern crate alloc;

    #[test]
    fn warmup_holds_neutral() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let f = flat_frame(64);
        let out = t.step(&f, 33);
        assert_eq!(out.motion, Motion::Warmup);
        assert_eq!(out.target, Pose::NEUTRAL);
    }

    #[test]
    fn no_motion_holds_pose_until_idle_timeout() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let f = flat_frame(64);
        let _ = t.step(&f, 33); // warmup
        // Same frame again: no motion. Should be Holding for the first
        // 3 s (idle_timeout_ms = 3000 in DEFAULT).
        let out = t.step(&f, 33);
        assert_eq!(out.motion, Motion::Holding);
        assert_eq!(out.target, Pose::NEUTRAL);
    }

    #[test]
    fn idle_timeout_returns_toward_neutral() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        // Seed the target away from NEUTRAL so the return-to-centre
        // slew has somewhere to slew *to*. Field access works because
        // the test lives inside the crate.
        t.target_pose = Pose::new(20.0, 10.0);

        let blank = flat_frame(64);
        let _ = t.step(&blank, 33); // warmup, prev grid populated

        // 4 s of no-motion steps at 33 ms each → past the 3 s timeout.
        let mut last_target = t.target_pose;
        for _ in 0..130 {
            let out = t.step(&blank, 33);
            last_target = out.target;
        }
        assert!(
            last_target.pan_deg.abs() < 20.0,
            "expected pan to slew below seed, got {}",
            last_target.pan_deg,
        );
        assert!(
            last_target.tilt_deg.abs() < 10.0,
            "expected tilt to slew below seed, got {}",
            last_target.tilt_deg,
        );
    }

    #[test]
    fn patch_appears_right_pans_head_right() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);

        // Frame 1: blank (warmup).
        let blank = flat_frame(20);
        let _ = t.step(&blank, 33);

        // Frame 2: bright 80×80 patch on the right edge. Fired cells
        // are exactly where the new content appeared — block columns
        // 6-7 — so the centroid lives there too.
        let mut f2 = flat_frame(20);
        paint_patch(&mut f2, 240, 80, 80, 80, 230);
        let out = t.step(&f2, 33);

        match out.motion {
            Motion::Tracking => {}
            other => panic!("expected Tracking, got {other:?}"),
        }
        let (nx, _ny) = out.centroid.unwrap();
        assert!(nx > 0.0, "expected centroid right-of-centre, got nx={nx}");
        assert!(out.target.pan_deg > 0.0);
        assert!(out.target.pan_deg <= TrackerConfig::DEFAULT.max_step_deg);
    }

    #[test]
    fn patch_appears_high_tilts_head_up() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let blank = flat_frame(20);
        let _ = t.step(&blank, 33);

        // 80×80 patch up high (top of frame), centred horizontally.
        let mut f2 = flat_frame(20);
        paint_patch(&mut f2, 120, 0, 80, 80, 230);
        let out = t.step(&f2, 33);

        assert_eq!(out.motion, Motion::Tracking);
        let (_nx, ny) = out.centroid.unwrap();
        assert!(ny < 0.0, "expected centroid above centre, got ny={ny}");
        // Negative ny → tilt up → positive tilt_deg.
        assert!(
            out.target.tilt_deg > 0.0,
            "expected upward tilt, got tilt_deg={}",
            out.target.tilt_deg,
        );
    }

    #[test]
    fn patch_appears_low_clamps_tilt_to_zero() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let blank = flat_frame(20);
        let _ = t.step(&blank, 33);

        let mut f2 = flat_frame(20);
        paint_patch(&mut f2, 120, 160, 80, 80, 230);
        let out = t.step(&f2, 33);

        assert_eq!(out.motion, Motion::Tracking);
        let (_nx, ny) = out.centroid.unwrap();
        assert!(ny > 0.0, "expected centroid below centre, got ny={ny}");
        // Positive ny would command negative tilt (look down), but
        // MIN_TILT_DEG = 0 so Pose::clamped pins it at 0.
        assert_eq!(out.target.tilt_deg, 0.0);
    }

    #[test]
    fn global_lighting_change_is_rejected() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let dark = flat_frame(10);
        let bright = flat_frame(220);
        let _ = t.step(&dark, 33); // warmup
        let out = t.step(&bright, 33);
        assert_eq!(
            out.motion,
            Motion::GlobalEvent,
            "expected GlobalEvent on whole-frame brightness flip, got {:?}",
            out.motion,
        );
        assert_eq!(out.target, Pose::NEUTRAL);
    }

    #[test]
    fn dead_zone_suppresses_centred_motion() {
        let mut cfg = TrackerConfig::DEFAULT;
        cfg.dead_zone = 0.5; // wide dead zone for the test
        let mut t = Tracker::new(cfg);

        let blank = flat_frame(20);
        let _ = t.step(&blank, 33);

        // Patch appears centred — fired cells cluster around the frame
        // centre, so the normalised centroid sits inside the wide dead
        // zone and the target pose isn't nudged.
        let mut f2 = flat_frame(20);
        paint_patch(&mut f2, 130, 90, 60, 60, 230);
        let out = t.step(&f2, 33);

        assert_eq!(out.motion, Motion::Tracking);
        assert_eq!(out.target, Pose::NEUTRAL);
    }

    #[test]
    fn reset_clears_state() {
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let f = flat_frame(64);
        let _ = t.step(&f, 33);
        t.reset();
        let out = t.step(&f, 33);
        assert_eq!(out.motion, Motion::Warmup);
    }

    #[test]
    fn target_pose_saturates_at_pan_clamp() {
        // Alternate blank ↔ patch-far-right so every transition fires
        // cells exclusively on the right side of the frame, biasing
        // pan deltas positive every step. Verify pan saturates at
        // MAX_PAN_DEG instead of running away.
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        let blank = flat_frame(20);
        let mut patch_right = flat_frame(20);
        paint_patch(&mut patch_right, 240, 80, 80, 80, 230);

        let _ = t.step(&blank, 33);
        for _ in 0..50 {
            let _ = t.step(&patch_right, 33);
            let _ = t.step(&blank, 33);
        }
        // Should have hit the pan clamp.
        assert!(
            (t.target_pose().pan_deg - stackchan_core::MAX_PAN_DEG).abs() < 0.01,
            "expected pan saturated at MAX_PAN_DEG, got {}",
            t.target_pose().pan_deg,
        );
        assert!(t.target_pose().tilt_deg <= stackchan_core::MAX_TILT_DEG);
        assert!(t.target_pose().tilt_deg >= stackchan_core::MIN_TILT_DEG);
    }
}
