//! Sensor inputs feeding the entity's world model.
//!
//! [`Perception`] owns every reading the firmware's per-peripheral
//! tasks publish via Signal channels. Modifiers in [`Phase::Affect`]
//! and [`Phase::Audio`] read these to react; nothing here directly
//! affects the rendered face — translation to visible state happens
//! through the emotion model and expression modifiers.
//!
//! Sensor freshness: each `Option<…>` field is `None` before the first
//! successful read and `Some(value)` after; the firmware never clears
//! these back to `None`. Modifiers that need stale-value detection
//! must track their own last-read timestamp via the entity's
//! [`crate::entity::Tick`].
//!
//! Future v2.x extensions: a processed `WorldModel` sub-struct
//! (smoothed velocities, sound-source bearing, gaze targets) sits on
//! top of these raw readings and is what cognition consumes.
//!
//! [`Phase::Affect`]: crate::app::Phase::Affect
//! [`Phase::Audio`]: crate::app::Phase::Audio

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
        }
    }
}
