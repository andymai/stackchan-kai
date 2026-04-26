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

/// One-frame summary of the firmware-side camera tracker's analysis.
///
/// The firmware's `tracker` crate runs the block-grid motion analysis
/// inside the camera task and publishes one of these per processed
/// frame. The engine consumes it via `entity.perception.tracking`
/// (added in the next PR) and decides what to do — see the
/// `TrackingFromCamera` `Phase::Perception` modifier and the
/// `AttentionFromTracking` `Phase::Cognition` modifier.
///
/// The shape mirrors `tracker::Outcome` minus the centroid, which is
/// already baked into `target_pose` by the tracker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackingObservation {
    /// Where the tracker thinks the head should look — already through
    /// the tracker's dead zone, P-gain, slew limit, and clamp.
    pub target_pose: Pose,
    /// Number of grid cells whose per-block luma delta exceeded the
    /// tracker's threshold this frame. `0` during warmup or genuine
    /// stillness; saturates at the grid's cell count.
    pub fired_cells: u16,
    /// Classification of this analysis step.
    pub motion: TrackingMotion,
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
        }
    }
}
