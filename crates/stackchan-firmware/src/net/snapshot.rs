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
use stackchan_core::Emotion;
use stackchan_core::head::Pose;

/// Battery snapshot — published by the power task.
#[derive(Debug, Clone, Copy, Default)]
pub struct BatterySnapshot {
    /// Estimated state-of-charge percentage (0..=100), or `None`
    /// before the first reading lands.
    pub percent: Option<u8>,
    /// Battery voltage in millivolts.
    pub voltage_mv: Option<u16>,
}

/// Wi-Fi link snapshot — published by the wifi task.
#[derive(Debug, Clone, Copy, Default)]
pub struct WifiSnapshot {
    /// Whether the controller currently reports a Connected link.
    pub connected: bool,
    /// IPv4 address assigned by DHCP, if any.
    pub ip: Option<Ipv4Addr>,
}

/// Composite avatar state surfaced via `GET /state`.
#[derive(Debug, Clone, Copy)]
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
}

impl Default for AvatarSnapshot {
    fn default() -> Self {
        Self {
            emotion: Emotion::Neutral,
            head_pose: Pose::default(),
            head_actual: None,
            battery: BatterySnapshot::default(),
            wifi: WifiSnapshot::default(),
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
    }));

/// Replace the avatar/head fields. Called per render tick.
pub fn update_avatar(emotion: Emotion, head_pose: Pose, head_actual: Option<Pose>) {
    AVATAR_SNAPSHOT.lock(|cell| {
        let mut s = cell.get();
        s.emotion = emotion;
        s.head_pose = head_pose;
        s.head_actual = head_actual;
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

/// Read the current snapshot. Cheap enough to call per HTTP request.
#[must_use]
pub fn read() -> AvatarSnapshot {
    AVATAR_SNAPSHOT.lock(core::cell::Cell::get)
}
