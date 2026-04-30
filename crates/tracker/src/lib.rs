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

pub mod cascade;
mod cascade_data;
mod luma;

pub use cascade_data::FRONTAL_FACE;

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
    /// Length of the per-cell temporal window used to suppress
    /// single-frame noise. A cell only counts as fired if it crossed
    /// `block_threshold` on at least [`Self::temporal_required`] of
    /// the most recent `temporal_window` frames. `1` disables the
    /// filter (every frame stands alone). Capped at `8`.
    pub temporal_window: u8,
    /// Number of fires required within [`Self::temporal_window`] to
    /// count a cell as fired this step. Must satisfy
    /// `1 <= temporal_required <= temporal_window`.
    pub temporal_required: u8,
    /// Minimum cell count of a connected blob. After temporal
    /// filtering, fired cells are grouped into 4-connected blobs;
    /// blobs smaller than this are dropped (treated as scatter
    /// noise). `1` disables the filter (every fired cell is its own
    /// candidate).
    pub min_blob_cells: u16,
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
    /// Single-pole low-pass blend on the *published* target pose.
    /// `1.0` (default) is a no-op — the published value matches the
    /// internal P-gain accumulator one-for-one. Lower values add
    /// inertia: `next = alpha * raw + (1 - alpha) * prev`. Useful for
    /// taming residual centroid jitter that the per-step P-gain still
    /// passes through.
    ///
    /// Stacks with the engine-side smoothing in
    /// `stackchan_core::modifiers::HeadFromAttention` (which applies
    /// its own α=0.22 EMA to the operator-visible head pose). Most
    /// operators won't need to lower this; benches sometimes do for
    /// cleaner visualisations. Clamped to `[0.05, 1.0]` at use sites
    /// to avoid the "head freezes forever" degenerate case at α=0.
    pub target_smoothing_alpha: f32,
}

impl TrackerConfig {
    /// Defaults tuned for QVGA GC0308 → `SCServo` head on a CoreS3.
    ///
    /// 8×6 grid (40×40 pixel blocks), 8 % per-block luma threshold,
    /// 50 % global-event ceiling, ≥2 cells required, ≥2-cell blob
    /// filter, P=0.4, ±10 % dead zone, 4 °/frame slew, 3 s idle
    /// timeout, 1 °/step return-to-centre.
    ///
    /// `block_threshold` was retuned again from 12 % → 8 % when the
    /// face-tracking face-bench landed: at typical 30–60 cm desk
    /// distance natural upper-body motion produces only 1–2 fired
    /// blocks at the older threshold, so the cascade never got a
    /// candidate to score. 8 % + `min_fired_cells = 2` catches that
    /// motion while still rejecting single-cell scatter and
    /// monitor-flicker shadows. `min_blob_cells = 2` keeps the
    /// connected-component filter requiring a coherent blob (not
    /// scattered single cells).
    ///
    /// The temporal filter is **off** by default
    /// (`temporal_window: 1` collapses the per-cell history to "this
    /// frame only"). Frame-differencing only fires once when a new
    /// object appears and stays still, so a 3-of-5-frames temporal
    /// filter would block stationary-presence detection.
    pub const DEFAULT: Self = Self {
        frame_width: 320,
        frame_height: 240,
        blocks_x: 8,
        blocks_y: 6,
        subsample_step: 2,
        fov_h_deg: 62.0,
        fov_v_deg: 49.0,
        block_threshold: 0.08,
        min_fired_cells: 2,
        max_fired_fraction: 0.50,
        temporal_window: 1,
        temporal_required: 1,
        min_blob_cells: 2,
        p_gain: 0.4,
        dead_zone: 0.10,
        max_step_deg: 4.0,
        idle_timeout_ms: 3_000,
        idle_step_deg: 1.0,
        flip_x: false,
        flip_y: false,
        target_smoothing_alpha: 1.0,
    };
}

/// Maximum number of distinct connected-blob candidates emitted per
/// [`Tracker::step`]. Excess blobs are dropped (largest-first).
pub const MAX_CANDIDATES: usize = 4;

/// Maximum value of [`TrackerConfig::temporal_window`]. Bounds the
/// per-cell history buffer baked into the [`Tracker`].
pub const MAX_TEMPORAL_WINDOW: usize = 8;

/// One detected motion blob, after temporal filtering + connected-
/// component grouping. Engine cognition layer arbitrates among these
/// to pick the focus target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetCandidate {
    /// Normalised centroid in `[-1.0, 1.0]` per axis. `(0, 0)` is
    /// frame centre.
    pub centroid: (f32, f32),
    /// Number of grid cells in this connected blob.
    pub cell_count: u16,
}

/// Outcome of one [`Tracker::step`] call.
///
/// Returned for diagnostic logging in the bench example and for unit
/// tests. The new target pose is always available as [`Outcome::target`];
/// [`Outcome::motion`] distinguishes the *why*.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Total number of grid cells that survived per-cell temporal
    /// filtering this step (i.e. fired on at least
    /// `TrackerConfig::temporal_required` of the last
    /// `temporal_window` frames). Capped at `blocks_x * blocks_y`.
    pub fired_cells: u16,
    /// Normalised centroid in `[-1.0, 1.0]` per axis when motion was
    /// detected, else `None`. Centroid of all fired cells across all
    /// blobs (the legacy single-target mode); engine cognition can
    /// instead arbitrate over [`Self::candidates`] for richer picks.
    pub centroid: Option<(f32, f32)>,
    /// Classification of this step.
    pub motion: Motion,
    /// New target pose after this step. Always within
    /// [`Pose::clamped`]'s safe range.
    pub target: Pose,
    /// Per-blob detections after connected-component labelling on the
    /// temporally-filtered fired-cell grid. Sorted by `cell_count`
    /// descending. Capped at [`MAX_CANDIDATES`]; smaller blobs are
    /// dropped. Empty on `Warmup` / `GlobalEvent` / no-motion.
    pub candidates: heapless::Vec<TargetCandidate, MAX_CANDIDATES>,
    /// Whether the face cascade fired on any candidate ROI this step.
    /// `false` when the cascade isn't run (the [`Tracker::step`] path
    /// never sets this; the firmware camera task patches the outcome
    /// after calling [`cascade::Cascade::scan_around_centroid`]).
    pub face_present: bool,
    /// Centroid of the highest-scoring face detection in normalised
    /// frame coordinates `[-1, 1]`. `None` when no face was scored or
    /// the cascade didn't fire.
    pub face_centroid: Option<(f32, f32)>,
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
    /// Per-cell ring buffer of fire decisions over the last
    /// `MAX_TEMPORAL_WINDOW` frames. Each `u8` is a bitfield where
    /// bit `i` is `1` iff the cell crossed `block_threshold` `i`
    /// frames ago. The temporal filter is `popcount(bits & mask) >=
    /// temporal_required` where `mask = (1 << temporal_window) - 1`.
    fire_history: [u8; MAX_BLOCKS],
    /// Internal P-gain accumulator. Each tracking step nudges this by
    /// the per-axis delta computed from the centroid offset; idle
    /// slew also operates on it. Always inside the [`Pose::clamped`]
    /// safe range. **Not** the value emitted in [`Outcome::target`] —
    /// see [`Self::published_pose`].
    target_pose: Pose,
    /// Smoothed copy of [`Self::target_pose`] published in
    /// [`Outcome::target`] and via [`Self::target_pose`]. EMA with
    /// [`TrackerConfig::target_smoothing_alpha`]; identical to
    /// `target_pose` when alpha is `1.0` (the default).
    published_pose: Pose,
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
            fire_history: [0; MAX_BLOCKS],
            target_pose: Pose::NEUTRAL,
            published_pose: Pose::NEUTRAL,
            idle_ms: 0,
        }
    }

    /// Borrow the active config.
    #[must_use]
    pub const fn config(&self) -> &TrackerConfig {
        &self.config
    }

    /// Most-recently commanded (smoothed) target pose. Equal to the
    /// internal accumulator when [`TrackerConfig::target_smoothing_alpha`]
    /// is `1.0`; otherwise a single-pole EMA of it. Returns the
    /// `published_pose` field, not the internal `target_pose`
    /// accumulator — the field of that name is the EMA's input,
    /// while callers of this getter want the EMA's output.
    #[must_use]
    #[allow(
        clippy::misnamed_getters,
        reason = "the `target pose` exposed to consumers is the smoothed (published) value; \
                  the internal `target_pose` field is the unsmoothed accumulator that \
                  feeds the EMA. Renaming the field would touch every step branch for no \
                  reader-facing benefit."
    )]
    pub const fn target_pose(&self) -> Pose {
        self.published_pose
    }

    /// Refresh [`Self::published_pose`] from [`Self::target_pose`] via
    /// the configured EMA, then return it. Called once per
    /// `Outcome` construction so smoothing keeps converging during
    /// `Holding` / `Returning` even when the accumulator hasn't moved.
    /// `α >= 1.0` (the default) is a one-for-one copy.
    const fn publish(&mut self) -> Pose {
        let alpha = self.config.target_smoothing_alpha.clamp(0.05, 1.0);
        if alpha >= 1.0 {
            self.published_pose = self.target_pose;
        } else {
            // Both inputs are already inside `Pose::clamped`'s safe
            // range (the accumulator + every published_pose update
            // route through it), and convex combinations of points
            // in the safe range stay in the safe range — no extra
            // clamp needed here.
            let inv = 1.0 - alpha;
            let pan = inv * self.published_pose.pan_deg + alpha * self.target_pose.pan_deg;
            let tilt = inv * self.published_pose.tilt_deg + alpha * self.target_pose.tilt_deg;
            self.published_pose = Pose::new(pan, tilt);
        }
        self.published_pose
    }

    /// Forget the previous frame and reset the target to `NEUTRAL`.
    /// The next [`Self::step`] will report [`Motion::Warmup`].
    pub const fn reset(&mut self) {
        self.have_prev = false;
        self.target_pose = Pose::NEUTRAL;
        self.published_pose = Pose::NEUTRAL;
        self.idle_ms = 0;
        // Reset the per-cell history so a re-entry starts fresh
        // (matches "the next step reports Warmup" contract).
        let mut i = 0;
        while i < MAX_BLOCKS {
            self.fire_history[i] = 0;
            i += 1;
        }
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
        clippy::too_many_lines,
        reason = "block-grid arithmetic: cell counts ≤ MAX_BLOCKS (256) so \
                  usize→u32 / u32→f32 casts cannot lose information; the \
                  f32→u16 casts are bounded by inputs that have already been \
                  clamped to [0, 255] / [0, total_cells]; per-axis \
                  `nx`/`ny`/`*_eff` pairs are the algorithm's natural names; \
                  step composes 7 sequential pipeline stages (luma fill → \
                  raw fire → temporal → CCL → blob filter → centroid → pose) \
                  that are clearer inline than fragmented across helpers"
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
                target: self.publish(),
                candidates: heapless::Vec::new(),
                face_present: false,
                face_centroid: None,
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
                target: self.publish(),
                candidates: heapless::Vec::new(),
                face_present: false,
                face_centroid: None,
            };
        }

        let threshold_u = ((cfg.block_threshold.clamp(0.0, 1.0)) * 255.0) as u16;
        let max_fired = ((cfg.max_fired_fraction.clamp(0.0, 1.0)) * total as f32) as u16;

        // Temporal filter parameters. Clamp the window to
        // MAX_TEMPORAL_WINDOW (8) so the bitfield fits in a u8; clamp
        // `required` to at least 1 and at most `window` so the
        // popcount comparison can fire.
        let temporal_window = cfg.temporal_window.clamp(1, MAX_TEMPORAL_WINDOW as u8);
        let temporal_required = cfg.temporal_required.clamp(1, temporal_window);
        let history_mask: u8 = if temporal_window >= 8 {
            0xFF
        } else {
            (1u8 << temporal_window) - 1
        };

        // Compute raw per-cell fire bits, advance the per-cell
        // history bitfield, and apply the temporal filter.
        let mut filtered: [bool; MAX_BLOCKS] = [false; MAX_BLOCKS];
        let mut fired_cells: u16 = 0;
        for iy in 0..usize::from(by) {
            for ix in 0..usize::from(bx) {
                let idx = iy * usize::from(bx) + ix;
                let curr = u16::from(curr_grid[idx]);
                let prev = u16::from(self.prev_grid[idx]);
                let delta = curr.abs_diff(prev);
                let raw_fired = delta > threshold_u;
                // Shift the history left by 1, drop bits beyond the
                // window, OR in the new fire bit.
                let shifted = (self.fire_history[idx] << 1) & history_mask;
                let new_history = shifted | u8::from(raw_fired);
                self.fire_history[idx] = new_history;
                if new_history.count_ones() >= u32::from(temporal_required) {
                    filtered[idx] = true;
                    fired_cells += 1;
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
                target: self.publish(),
                candidates: heapless::Vec::new(),
                face_present: false,
                face_centroid: None,
            };
        }

        // Connected-component labelling on the filtered grid. Each
        // blob below `min_blob_cells` is dropped (treated as scatter
        // noise that the temporal filter happened to let through).
        let mut blobs = label_blobs(&filtered, bx, by);
        blobs.retain(|b| b.cell_count >= cfg.min_blob_cells);

        // Aggregate centroid + cell count over surviving blobs only;
        // this is the legacy single-pose-target signal that tracker's
        // pose math uses. Engine cognition can instead arbitrate over
        // `candidates` for richer focus selection.
        let mut sum_x: u32 = 0;
        let mut sum_y: u32 = 0;
        let mut valid_cells: u16 = 0;
        for blob in &blobs {
            sum_x += u32::from(blob.sum_x);
            sum_y += u32::from(blob.sum_y);
            valid_cells += blob.cell_count;
        }

        if valid_cells < cfg.min_fired_cells {
            return self.no_motion_outcome(dt_ms, valid_cells);
        }

        // Build the candidates list: blobs sorted by cell_count
        // descending, capped at MAX_CANDIDATES, normalised centroids.
        let candidates = build_candidates(&mut blobs, bx, by, cfg.flip_x, cfg.flip_y);

        // Centroid in block-index space, then normalised to [-1, 1].
        let (cell_cx, cell_cy) = (
            (sum_x as f32) / f32::from(valid_cells),
            (sum_y as f32) / f32::from(valid_cells),
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
            fired_cells: valid_cells,
            centroid: Some((nx, ny)),
            motion: Motion::Tracking,
            target: self.publish(),
            candidates,
            face_present: false,
            face_centroid: None,
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
                target: self.publish(),
                candidates: heapless::Vec::new(),
                face_present: false,
                face_centroid: None,
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
            target: self.publish(),
            candidates: heapless::Vec::new(),
            face_present: false,
            face_centroid: None,
        }
    }
}

/// One blob from connected-component labelling. Internal to the
/// step's bookkeeping; a final pass converts surviving blobs into
/// public [`TargetCandidate`]s with normalised centroids.
#[derive(Debug, Clone, Copy)]
struct Blob {
    /// Sum of x-indices of cells in this blob.
    sum_x: u16,
    /// Sum of y-indices of cells in this blob.
    sum_y: u16,
    /// Number of cells in this blob.
    cell_count: u16,
}

/// Run 4-connected CCL on the filtered fired-cell grid via iterative
/// flood-fill. Visits each fired cell exactly once. Stack capacity
/// equals [`MAX_BLOCKS`] — sufficient because the worst-case blob
/// fills the entire grid.
fn label_blobs(filtered: &[bool; MAX_BLOCKS], bx: u16, by: u16) -> heapless::Vec<Blob, MAX_BLOCKS> {
    let mut visited = [false; MAX_BLOCKS];
    let mut blobs: heapless::Vec<Blob, MAX_BLOCKS> = heapless::Vec::new();
    let cols = usize::from(bx);
    let rows = usize::from(by);
    for sy in 0..rows {
        for sx in 0..cols {
            let seed_idx = sy * cols + sx;
            if !filtered[seed_idx] || visited[seed_idx] {
                continue;
            }
            // Iterative flood-fill from (sx, sy).
            let mut stack: heapless::Vec<(u8, u8), MAX_BLOCKS> = heapless::Vec::new();
            #[allow(
                clippy::cast_possible_truncation,
                reason = "bx/by ≤ MAX_BLOCKS_X/Y ≤ 16, fits in u8 trivially"
            )]
            let _ = stack.push((sx as u8, sy as u8));
            visited[seed_idx] = true;
            let mut blob = Blob {
                sum_x: 0,
                sum_y: 0,
                cell_count: 0,
            };
            while let Some((cx, cy)) = stack.pop() {
                blob.sum_x += u16::from(cx);
                blob.sum_y += u16::from(cy);
                blob.cell_count += 1;
                // 4-neighbours.
                let neighbours = [
                    (cx.wrapping_sub(1), cy, cx > 0),
                    (cx + 1, cy, usize::from(cx) + 1 < cols),
                    (cx, cy.wrapping_sub(1), cy > 0),
                    (cx, cy + 1, usize::from(cy) + 1 < rows),
                ];
                for (nx, ny, in_bounds) in neighbours {
                    if !in_bounds {
                        continue;
                    }
                    let nidx = usize::from(ny) * cols + usize::from(nx);
                    if filtered[nidx] && !visited[nidx] {
                        visited[nidx] = true;
                        let _ = stack.push((nx, ny));
                    }
                }
            }
            // `blobs` is bounded by MAX_BLOCKS — push always succeeds
            // because we visit each cell once and a blob owns ≥1 cell.
            let _ = blobs.push(blob);
        }
    }
    blobs
}

/// Sort `blobs` by `cell_count` descending and convert the top
/// [`MAX_CANDIDATES`] into public [`TargetCandidate`]s with
/// normalised centroids in `[-1, 1]`. Honours `flip_x` / `flip_y` so
/// engine-side arbitration sees centroids in the same coordinate
/// frame the tracker's pose math uses.
#[allow(
    clippy::cast_precision_loss,
    reason = "cell counts ≤ MAX_BLOCKS = 256 ≪ 2^24, exact in f32"
)]
fn build_candidates(
    blobs: &mut heapless::Vec<Blob, MAX_BLOCKS>,
    bx: u16,
    by: u16,
    flip_x: bool,
    flip_y: bool,
) -> heapless::Vec<TargetCandidate, MAX_CANDIDATES> {
    blobs.sort_unstable_by_key(|b| core::cmp::Reverse(b.cell_count));
    let half_x = (f32::from(bx) - 1.0) * 0.5;
    let half_y = (f32::from(by) - 1.0) * 0.5;
    let mut out: heapless::Vec<TargetCandidate, MAX_CANDIDATES> = heapless::Vec::new();
    for blob in blobs.iter().take(MAX_CANDIDATES) {
        let cx = f32::from(blob.sum_x) / f32::from(blob.cell_count);
        let cy = f32::from(blob.sum_y) / f32::from(blob.cell_count);
        let mut nx = (cx - half_x) / half_x.max(0.5);
        let mut ny = (cy - half_y) / half_y.max(0.5);
        if flip_x {
            nx = -nx;
        }
        if flip_y {
            ny = -ny;
        }
        let _ = out.push(TargetCandidate {
            centroid: (nx, ny),
            cell_count: blob.cell_count,
        });
    }
    out
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
    fn default_alpha_publishes_accumulator_one_for_one() {
        // DEFAULT alpha = 1.0 → published_pose mirrors the internal
        // target_pose accumulator on every step, no inertia. Pin
        // because every existing tuning assumes this behaviour.
        let mut t = Tracker::new(TrackerConfig::DEFAULT);
        t.target_pose = Pose::new(10.0, 5.0);
        let f = flat_frame(64);
        let _ = t.step(&f, 33); // warmup populates prev grid
        let out = t.step(&f, 33);
        assert!(
            (out.target.pan_deg - t.target_pose.pan_deg).abs() < f32::EPSILON,
            "alpha=1.0 should publish target_pose verbatim; got {} vs {}",
            out.target.pan_deg,
            t.target_pose.pan_deg
        );
        assert!(
            (out.target.tilt_deg - t.target_pose.tilt_deg).abs() < f32::EPSILON,
            "alpha=1.0 should publish target_pose verbatim; got {} vs {}",
            out.target.tilt_deg,
            t.target_pose.tilt_deg
        );
    }

    #[test]
    fn alpha_below_one_smooths_published_toward_target_over_frames() {
        // With alpha=0.5 each step halves the gap to target_pose.
        // After 4 steps the published_pose should be within ~6 % of
        // the target (1 - 0.5^4 = 0.9375).
        let mut cfg = TrackerConfig::DEFAULT;
        cfg.target_smoothing_alpha = 0.5;
        let mut t = Tracker::new(cfg);
        t.target_pose = Pose::new(20.0, 0.0); // accumulator anchor
        let f = flat_frame(64);
        let _ = t.step(&f, 33); // warmup; publishes once

        let after_warmup = t.target_pose();
        // After warmup publish: 0.5 * 0 + 0.5 * 20 = 10.0 (one EMA step).
        assert!(
            (after_warmup.pan_deg - 10.0).abs() < 0.01,
            "expected ~10.0 after first publish, got {}",
            after_warmup.pan_deg
        );

        // Three more no-motion steps. Each Holding-path call invokes
        // publish() and shrinks the gap by half: 10 → 15 → 17.5 → 18.75.
        for _ in 0..3 {
            let _ = t.step(&f, 33);
        }
        let pan = t.target_pose().pan_deg;
        assert!(
            (pan - 18.75).abs() < 0.01,
            "expected ~18.75 after 4 EMA steps at alpha=0.5, got {pan}"
        );
    }

    #[test]
    fn alpha_clamped_so_smoothing_never_stalls_at_zero() {
        // alpha = 0.0 would make published_pose immune to any change
        // in target_pose forever (a UX bug masquerading as a config
        // value). The publish() guard clamps to 0.05 so the EMA
        // converges, slowly. Pin: pan reaches at least 1° within
        // 50 steps from a 20° accumulator.
        let mut cfg = TrackerConfig::DEFAULT;
        cfg.target_smoothing_alpha = 0.0;
        let mut t = Tracker::new(cfg);
        t.target_pose = Pose::new(20.0, 0.0);
        let f = flat_frame(64);
        let _ = t.step(&f, 33); // warmup → first publish
        for _ in 0..49 {
            let _ = t.step(&f, 33);
        }
        let pan = t.target_pose().pan_deg;
        assert!(
            pan > 1.0,
            "alpha clamped at 0.05 should let EMA progress; pan = {pan} after 50 steps"
        );
    }

    #[test]
    fn reset_zeros_published_pose() {
        // After a reset, both the accumulator and the published
        // (smoothed) value should land at NEUTRAL — no leftover
        // inertia from the previous run.
        let mut cfg = TrackerConfig::DEFAULT;
        cfg.target_smoothing_alpha = 0.5;
        let mut t = Tracker::new(cfg);
        t.target_pose = Pose::new(15.0, 8.0);
        let f = flat_frame(64);
        let _ = t.step(&f, 33);
        assert!(t.target_pose().pan_deg.abs() > 0.0);

        t.reset();
        assert_eq!(t.target_pose(), Pose::NEUTRAL);
        assert_eq!(t.target_pose, Pose::NEUTRAL);
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
