//! Read-side avatar snapshot for the HTTP control plane.
//!
//! The render task updates [`AVATAR_SNAPSHOT`] once per frame; the
//! HTTP handler reads it via a critical-section borrow without
//! consuming any of the existing [`embassy_sync::signal::Signal`]
//! channels (which are single-consumer / latest-wins by design — a
//! second `try_take` from the HTTP task would steal sensor data
//! from the render task and lose frames).

use core::net::Ipv4Addr;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::Instant as EmbassyInstant;
use stackchan_core::head::Pose;
use stackchan_core::{Decorator, Emotion, FaceGeometry, Mood};
use stackchan_net::config::AudioConfig;

/// Battery snapshot — published by the power task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatterySnapshot {
    /// Estimated state-of-charge percentage (0..=100), or `None`
    /// before the first reading lands.
    pub percent: Option<u8>,
    /// Battery voltage in millivolts.
    pub voltage_mv: Option<u16>,
}

/// Wi-Fi link snapshot — published by the wifi task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WifiSnapshot {
    /// Whether the controller currently reports a Connected link.
    pub connected: bool,
    /// IPv4 address assigned by DHCP, if any.
    pub ip: Option<Ipv4Addr>,
}

/// Composite avatar state surfaced via `GET /state`.
///
/// Only `PartialEq` (not `Eq`) because `head_pose` carries `f32`s.
/// Note: `==` returns `false` for NaN-vs-NaN; use [`Self::bit_eq`]
/// when "did anything change" matters more than IEEE 754 equality.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarSnapshot {
    /// Current emotion produced by the modifier pipeline.
    pub emotion: Emotion,
    /// Most recently commanded head pose.
    pub head_pose: Pose,
    /// Most recently observed servo pose, if available.
    pub head_actual: Option<Pose>,
    /// Power / battery telemetry.
    pub battery: BatterySnapshot,
    /// Wi-Fi link state.
    pub wifi: WifiSnapshot,
    /// Persisted audio settings (volume + mute) currently applied to
    /// the AW88298. Mirrored from `CONFIG_SNAPSHOT.audio` on every
    /// successful `POST /volume` / `POST /mute` so the dashboard's
    /// SSE stream picks up the change without a full settings re-fetch.
    pub audio: AudioConfig,
    /// Whether the LCD is currently showing the camera preview
    /// (`true`) instead of the avatar (`false`). Display-only —
    /// tracking still runs in either mode. Updated by every producer
    /// of [`crate::camera::CAMERA_MODE_SIGNAL`] (button long-press,
    /// HTTP `POST /camera/mode`, BLE view-service write) before the
    /// signal fires, so the snapshot stays canonical.
    pub camera_mode: bool,
    /// Active decorator overlay, if any. Mirrors
    /// `entity.face.decorator.kind`; the `expires_at` deadline is
    /// internal and not exposed.
    pub decorator: Option<Decorator>,
    /// Operator-selected energy baseline. Mirrors `entity.mind.mood`;
    /// HTTP `POST /mood` updates this through `MOOD_SIGNAL`.
    pub mood: Mood,
    /// Operator-selected face geometry preset. Mirrors
    /// `entity.face.geometry`; HTTP `POST /face-geometry` and MCP
    /// `set_face_geometry` update this through `FACE_GEOMETRY_SIGNAL`.
    pub face_geometry: FaceGeometry,
}

impl AvatarSnapshot {
    /// Bit-pattern equality across all `f32` fields, plus value
    /// equality on everything else. Unlike `==`, this returns `true`
    /// for NaN-vs-NaN with the same bit layout, which is what the
    /// "did anything change since last frame?" gate wants — IEEE
    /// 754 NaN-inequality would otherwise make every tick look
    /// changed when a pose value gets stuck at NaN.
    #[must_use]
    pub fn bit_eq(&self, other: &Self) -> bool {
        let pose_eq = |a: Pose, b: Pose| {
            a.pan_deg.to_bits() == b.pan_deg.to_bits()
                && a.tilt_deg.to_bits() == b.tilt_deg.to_bits()
        };
        let actual_eq = match (self.head_actual, other.head_actual) {
            (Some(a), Some(b)) => pose_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.emotion == other.emotion
            && pose_eq(self.head_pose, other.head_pose)
            && actual_eq
            && self.battery == other.battery
            && self.wifi == other.wifi
            && self.audio == other.audio
            && self.camera_mode == other.camera_mode
            && self.decorator == other.decorator
            && self.mood == other.mood
            && self.face_geometry == other.face_geometry
    }
}

impl Default for AvatarSnapshot {
    fn default() -> Self {
        Self {
            emotion: Emotion::Neutral,
            head_pose: Pose::default(),
            head_actual: None,
            battery: BatterySnapshot::default(),
            wifi: WifiSnapshot::default(),
            audio: AudioConfig::DEFAULT,
            camera_mode: false,
            decorator: None,
            mood: Mood::Neutral,
            face_geometry: FaceGeometry::Default,
        }
    }
}

/// The shared snapshot. Updated per render tick and on per-task
/// transitions; HTTP reads borrow non-destructively.
pub static AVATAR_SNAPSHOT: Mutex<CriticalSectionRawMutex, core::cell::Cell<AvatarSnapshot>> =
    Mutex::new(core::cell::Cell::new(AvatarSnapshot {
        emotion: Emotion::Neutral,
        head_pose: Pose {
            pan_deg: 0.0,
            tilt_deg: 0.0,
        },
        head_actual: None,
        battery: BatterySnapshot {
            percent: None,
            voltage_mv: None,
        },
        wifi: WifiSnapshot {
            connected: false,
            ip: None,
        },
        audio: AudioConfig::DEFAULT,
        camera_mode: false,
        decorator: None,
        mood: Mood::Neutral,
        face_geometry: FaceGeometry::Default,
    }));

/// Replace the avatar/head fields. Called per render tick.
pub fn update_avatar(
    emotion: Emotion,
    head_pose: Pose,
    head_actual: Option<Pose>,
    decorator: Option<Decorator>,
    mood: Mood,
    face_geometry: FaceGeometry,
) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.emotion = emotion;
        s.head_pose = head_pose;
        s.head_actual = head_actual;
        s.decorator = decorator;
        s.mood = mood;
        s.face_geometry = face_geometry;
        cell.set(s);
    });
}

/// Replace the battery fields. Called by the power task on each poll.
pub fn update_battery(percent: Option<u8>, voltage_mv: Option<u16>) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.battery = BatterySnapshot {
            percent,
            voltage_mv,
        };
        cell.set(s);
    });
}

/// Replace the Wi-Fi link fields. Called by the wifi task on link
/// transitions and by `net_runner` on DHCP lease changes.
pub fn update_wifi(connected: bool, ip: Option<Ipv4Addr>) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.wifi = WifiSnapshot { connected, ip };
        cell.set(s);
    });
}

/// Replace the audio (volume / mute) fields. Called once at boot
/// from the persisted config, and again after each successful
/// `POST /volume` or `POST /mute` so the SSE stream surfaces the
/// change.
pub fn update_audio(audio: AudioConfig) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.audio = audio;
        cell.set(s);
    });
}

/// Sensors snapshot — populated by the producer tasks via the
/// `update_*` mirrors below.
///
/// HTTP reads through [`read_sensors`] for `GET /sensors` without ever
/// touching the source `Signal` channels (which are single-consumer /
/// latest-wins by design — the render task already drains them).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SensorsSnapshot {
    /// Most recent IMU reading. `None` until the BMI270 init succeeds.
    pub imu: Option<ImuFields>,
    /// Most recent ambient-light reading in lux. `None` until the
    /// `LTR-553` publishes its first sample.
    pub ambient_lux: Option<f32>,
    /// Most recent microphone-RMS sample, normalised 0..=1. `0.0`
    /// before the audio task spins up (the audio loop publishes
    /// `AudioRms(0.0)` on DMA stalls too, so this overlaps).
    pub audio_rms: f32,
    /// Latest body-touch zones (left, centre, right) on the
    /// 0..=3 intensity scale. `None` until `Si12T` init reports a value.
    pub body_touch: Option<BodyTouchFields>,
}

/// Wire-format IMU sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImuFields {
    /// Accelerometer reading in g units.
    pub accel_g: (f32, f32, f32),
    /// Gyroscope reading in degrees per second.
    pub gyro_dps: (f32, f32, f32),
}

/// Wire-format body-touch reading. Mirrors `stackchan_core::BodyTouch`
/// but lives here so the HTTP layer can serialize it without pulling
/// in core's wider type surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BodyTouchFields {
    /// Left-zone intensity 0..=3.
    pub left: u8,
    /// Centre-zone intensity 0..=3.
    pub centre: u8,
    /// Right-zone intensity 0..=3.
    pub right: u8,
}

/// Backing static. Updates use cell-replace under a critical section
/// to match the rest of this module's `Cell<T>`-in-`Mutex` pattern.
pub static SENSORS_SNAPSHOT: Mutex<CriticalSectionRawMutex, core::cell::Cell<SensorsSnapshot>> =
    Mutex::new(core::cell::Cell::new(SensorsSnapshot {
        imu: None,
        ambient_lux: None,
        audio_rms: 0.0,
        body_touch: None,
    }));

/// Read the current sensors snapshot.
#[must_use]
pub fn read_sensors() -> SensorsSnapshot {
    SENSORS_SNAPSHOT.lock(core::cell::Cell::get)
}

/// Mirror an IMU sample. Called by the IMU task right after each
/// successful read.
pub fn update_imu(accel_g: (f32, f32, f32), gyro_dps: (f32, f32, f32)) {
    SENSORS_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.imu = Some(ImuFields { accel_g, gyro_dps });
        cell.set(s);
    });
}

/// Mirror the latest ambient-light reading.
pub fn update_ambient_lux(lux: f32) {
    SENSORS_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.ambient_lux = Some(lux);
        cell.set(s);
    });
}

/// Mirror the latest audio-RMS sample (already normalised 0..=1).
pub fn update_audio_rms(rms: f32) {
    SENSORS_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.audio_rms = rms;
        cell.set(s);
    });
}

/// Mirror the latest body-touch zone state.
pub fn update_body_touch(left: u8, centre: u8, right: u8) {
    SENSORS_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.body_touch = Some(BodyTouchFields {
            left,
            centre,
            right,
        });
        cell.set(s);
    });
}

/// Replace the camera-mode field.
///
/// Called by every producer of [`crate::camera::CAMERA_MODE_SIGNAL`]
/// (button long-press, HTTP `POST /camera/mode`, BLE view-service
/// write) so the snapshot flips before the signal fires — `GET
/// /state` and BLE notify clients see the new value without waiting
/// for the render loop to drain the signal.
pub fn update_camera_mode(active: bool) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.camera_mode = active;
        cell.set(s);
    });
}

/// Read the current snapshot. Cheap enough to call per HTTP request.
#[must_use]
pub fn read() -> AvatarSnapshot {
    AVATAR_SNAPSHOT.lock(core::cell::Cell::get)
}

/// Maximum simultaneous SSE subscribers. Matches the HTTP worker
/// pool size — each worker holds at most one subscriber, so SSE
/// connections beyond this aren't possible anyway.
pub const SSE_MAX_SUBSCRIBERS: usize = 4;

/// Multi-producer single-publisher channel that pushes the latest
/// [`AvatarSnapshot`] to SSE subscribers on `GET /state/stream`.
///
/// Capacity 1: latest-wins. If a subscriber lags it sees a `Lagged`
/// result and continues at the next message — operators don't need
/// every frame, just current state.
pub static SNAPSHOT_PUBSUB: PubSubChannel<
    CriticalSectionRawMutex,
    AvatarSnapshot,
    1,
    SSE_MAX_SUBSCRIBERS,
    1,
> = PubSubChannel::new();

/// Minimum interval between publishes. Render runs at ~30 Hz; we
/// throttle the SSE firehose to ~10 Hz so a slow client (or four
/// of them) can keep up without backpressure.
const PUBLISH_MIN_INTERVAL_MS: u64 = 100;

/// Last-published-at instant, in `embassy_time::Instant` ticks.
/// `None` until the first publish; `Some(ms)` thereafter so the
/// throttle gate doesn't have to reserve a magic value.
static LAST_PUBLISH_MS: Mutex<CriticalSectionRawMutex, core::cell::Cell<Option<u64>>> =
    Mutex::new(core::cell::Cell::new(None));

/// Last-published snapshot value, used to suppress duplicate
/// publishes when nothing visible changed between ticks.
static LAST_PUBLISHED: Mutex<CriticalSectionRawMutex, core::cell::Cell<Option<AvatarSnapshot>>> =
    Mutex::new(core::cell::Cell::new(None));

/// Publish the current snapshot to [`SNAPSHOT_PUBSUB`] subscribers.
///
/// Called once per render tick by the render task. Only publishes
/// when the snapshot has actually changed AND
/// [`PUBLISH_MIN_INTERVAL_MS`] has elapsed since the last publish.
///
/// Cheap when no subscribers are connected (the publisher's
/// `publish_immediate` returns immediately if the channel has no
/// listeners), so this stays out of the hot path of the
/// boot-only-no-Wi-Fi scenario.
pub fn publish_if_changed() {
    let now_ms = EmbassyInstant::now().as_millis();
    let last_ms = LAST_PUBLISH_MS.lock(core::cell::Cell::get);
    if let Some(last) = last_ms
        && now_ms.saturating_sub(last) < PUBLISH_MIN_INTERVAL_MS
    {
        return;
    }
    let current = read();
    // `bit_eq` instead of `!=`: NaN in f32 pose fields would otherwise
    // make every tick look changed under PartialEq.
    let changed = LAST_PUBLISHED.lock(|cell| cell.get().is_none_or(|prev| !prev.bit_eq(&current)));
    if !changed {
        return;
    }
    LAST_PUBLISH_MS.lock(|cell| cell.set(Some(now_ms)));
    LAST_PUBLISHED.lock(|cell| cell.set(Some(current)));
    let publisher = SNAPSHOT_PUBSUB.immediate_publisher();
    publisher.publish_immediate(current);
}
