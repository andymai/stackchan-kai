//! Sensor inputs feeding the entity's world model.
//!
//! [`Perception`] holds every reading the firmware's per-peripheral
//! tasks publish via Signal channels. Modifiers in [`Phase::Affect`]
//! and [`Phase::Audio`] read these; nothing here directly affects the
//! rendered face — translation to visible state happens through the
//! emotion model and expression modifiers.
//!
//! Each `Option<…>` field is `None` before the first successful read
//! and `Some(value)` after; the firmware never clears these back to
//! `None`. Modifiers that need stale-value detection must track their
//! own last-read timestamp via [`crate::entity::Tick`].
//!
//! [`Phase::Affect`]: crate::director::Phase::Affect
//! [`Phase::Audio`]: crate::director::Phase::Audio

use crate::head::Pose;

/// Per-zone body-touch intensity (back-of-head `Si12T` pads).
///
/// Each zone carries a 0..=3 intensity matching the chip's 2-bit
/// per-channel encoding (`0` = no touch, `1..=3` = touch with rising
/// firmness). Modifiers / skills do their own edge / gesture detection.
///
/// The intensity (vs a plain `bool`) is what `position()` and the
/// swipe state machine in [`crate::modifiers::IntentFromBodyTouch`] need —
/// reducing to `bool` would lose the centroid math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BodyTouch {
    /// Left zone intensity, `0..=3`.
    pub left: u8,
    /// Centre zone intensity, `0..=3`.
    pub centre: u8,
    /// Right zone intensity, `0..=3`.
    pub right: u8,
}

impl BodyTouch {
    /// `true` if any zone has non-zero intensity (matches upstream's
    /// `is_touched` heuristic of `intensity >= 1`).
    #[must_use]
    pub const fn any(&self) -> bool {
        self.left >= 1 || self.centre >= 1 || self.right >= 1
    }

    /// Centroid in `-100..=+100` (left-most → -100, centre → 0,
    /// right-most → +100), weighted by intensity. Returns `0` when
    /// no zones are touched. Used by gesture detection to recognise
    /// swipes as the touch centroid moves across the strip.
    #[must_use]
    pub const fn position(&self) -> i16 {
        let total = self.left as i16 + self.centre as i16 + self.right as i16;
        if total == 0 {
            return 0;
        }
        // Centre contributes 0; left = -100, right = +100. Max
        // numerator magnitude is 3 × 100 = 300; dividing by total
        // (≥ 1, ≤ 9) keeps the result in `-100..=+100` — well inside
        // i16 range.
        let weighted = (self.right as i16 - self.left as i16) * 100;
        weighted / total
    }
}

/// Maximum tracker candidates surfaced to the engine per frame.
/// Mirrors `tracker::MAX_CANDIDATES`; kept here so the engine doesn't
/// import the tracker crate just for the const.
pub const MAX_TRACKING_CANDIDATES: usize = 4;

/// Half the camera's horizontal FOV, in degrees.
///
/// Used by engagement modifiers to convert a normalised face centroid
/// (`-1..1`) into a head-pose target — the cascade emits centroids in
/// frame-normalised coordinates, but `Pose` is in degrees.
///
/// Mirrors `tracker::TrackerConfig::DEFAULT.fov_h_deg / 2.0`. The
/// `tracker` crate is firmware-side; pulling it in from
/// `stackchan-core` would leak `no_std` boundaries, so this constant
/// is duplicated. Audit with `rg HALF_FOV` if the camera lens
/// changes.
pub const HALF_FOV_H_DEG: f32 = 31.0;
/// Half the camera's vertical FOV, in degrees. See [`HALF_FOV_H_DEG`].
pub const HALF_FOV_V_DEG: f32 = 24.5;

/// One detected motion blob from the firmware tracker, after
/// temporal + connected-component filtering.
///
/// Mirrors `tracker::TargetCandidate` — the engine doesn't depend on
/// `tracker`, so we duplicate the shape. Used by
/// `AttentionFromTracking` for multi-target arbitration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetCandidate {
    /// Normalised centroid in `[-1, 1]` per axis. `(0, 0)` is frame
    /// centre. Already honours `flip_x` / `flip_y` from the tracker
    /// config.
    pub centroid: (f32, f32),
    /// Number of grid cells in this blob. Used as a saliency proxy
    /// (bigger blob = more interesting target).
    pub cell_count: u16,
}

/// One-frame summary of the firmware-side camera tracker's analysis.
///
/// The firmware's `tracker` crate runs the block-grid motion analysis
/// inside the camera task and publishes one of these per processed
/// frame. The engine consumes it via `entity.perception.tracking`
/// and decides what to do — see the `AttentionFromTracking`
/// `Phase::Cognition` modifier.
///
/// `target_pose` is the tracker's legacy single-target pose (centroid
/// of all valid blobs, slewed via dead zone + P-gain + step limit).
/// `candidates` is the post-CCL per-blob list — engine cognition can
/// arbitrate among these via saliency / novelty / habituation scoring
/// instead of trusting the aggregate centroid.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackingObservation {
    /// Where the tracker thinks the head should look — aggregate
    /// centroid of all surviving blobs, already through the tracker's
    /// dead zone, P-gain, slew limit, and clamp.
    pub target_pose: Pose,
    /// Total number of grid cells across all surviving blobs. `0`
    /// during warmup or genuine stillness; saturates at the grid's
    /// cell count.
    pub fired_cells: u16,
    /// Classification of this analysis step.
    pub motion: TrackingMotion,
    /// Per-blob detections after temporal filtering + CCL, sorted by
    /// `cell_count` descending. Cap [`MAX_TRACKING_CANDIDATES`].
    /// Empty on `Warmup` / `GlobalEvent` / no-motion.
    pub candidates: heapless::Vec<TargetCandidate, MAX_TRACKING_CANDIDATES>,
    /// Whether the firmware-side face cascade fired on any motion
    /// candidate this frame. `false` when cascade scoring is disabled
    /// or no motion was found. Engine cognition modifiers gate
    /// engagement state on this — observation-only in v0.x; behavioural
    /// effects ship in a follow-up PR.
    pub face_present: bool,
    /// Centroid of the highest-scoring face detection in normalised
    /// frame coordinates `[-1, 1]`. `None` when no face was scored or
    /// the cascade didn't fire. Provided alongside [`Self::face_present`]
    /// so engine cognition can centre attention on the face directly
    /// rather than the (potentially larger) motion blob.
    pub face_centroid: Option<(f32, f32)>,
}

/// Why the tracker chose its current target. Mirrors `tracker::Motion`
/// without depending on the tracker crate from `stackchan-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackingMotion {
    /// First frame after construction or `reset` — no previous frame
    /// to diff against. Target pose is unchanged from `Pose::NEUTRAL`.
    Warmup,
    /// Real motion detected; target pose was nudged toward it.
    Tracking,
    /// No motion this step but still inside the idle timeout window;
    /// target pose held at the last commanded value.
    Holding,
    /// Idle long enough to be slewing back toward `Pose::NEUTRAL`.
    Returning,
    /// Too many cells fired this frame — the tracker treated it as a
    /// global lighting change and held the target.
    GlobalEvent,
}

/// Raw sensor readings that drive reactive modifiers.
///
/// `Clone` only (not `Copy`) — `tracking` carries a `heapless::Vec`
/// of multi-target candidates which can't be `Copy`.
#[derive(Debug, Clone, PartialEq)]
pub struct Perception {
    /// Accelerometer reading in gravitational units `(x, y, z)`.
    /// Resting face-up on a flat surface reads `(0, 0, 1)`. Written by
    /// the firmware IMU task at ~100 Hz.
    pub accel_g: (f32, f32, f32),
    /// Gyroscope reading in degrees per second `(x, y, z)`. Zero at
    /// rest. Written by the firmware IMU task.
    pub gyro_dps: (f32, f32, f32),
    /// Ambient light level in lux, or `None` before the first
    /// successful LTR-553 read.
    pub ambient_lux: Option<f32>,
    /// Battery state-of-charge in percent (`0..=100`), or `None`
    /// before the first successful AXP2101 gauge read.
    pub battery_percent: Option<u8>,
    /// Whether the AXP2101 reports valid USB power on its VBUS input,
    /// or `None` before the first successful read.
    pub usb_power_present: Option<bool>,
    /// Latest microphone RMS amplitude, normalised against full-scale
    /// i16 (`0.0..=1.0`), or `None` before the audio task publishes
    /// its first window.
    pub audio_rms: Option<f32>,
    /// Per-zone body-touch state from the back-of-head `Si12T` pads,
    /// or `None` before the first successful read. Continuous state,
    /// not an edge — modifiers add their own edge detection if needed.
    pub body_touch: Option<BodyTouch>,
    /// Latest camera tracker observation (target pose + motion class +
    /// fired-cell count), or `None` before the camera task publishes
    /// its first frame. The firmware tracker runs at camera frame
    /// rate (~30 Hz); the engine drains into this field once per
    /// render tick (~30 Hz). Cognition modifiers read this to decide
    /// whether the avatar should latch attention to the moving
    /// target.
    pub tracking: Option<TrackingObservation>,
}

impl Default for Perception {
    fn default() -> Self {
        Self {
            // Resting face-up: gravity is +1 g along Z, no rotation.
            accel_g: (0.0, 0.0, 1.0),
            gyro_dps: (0.0, 0.0, 0.0),
            ambient_lux: None,
            battery_percent: None,
            usb_power_present: None,
            audio_rms: None,
            body_touch: None,
            tracking: None,
        }
    }
}
