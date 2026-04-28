//! BLE peripheral surface — advertise + GATT services + provisioning.
//!
//! On boot, [`ble_task`] takes ownership of the trouble-host
//! `ExternalController` (built from `esp_radio`'s `BleConnector`) and
//! drives an advertise → accept-connection → serve-GATT loop for the
//! firmware lifetime. The advertised local name is
//! `stackchan-XXXXXX`, where `XXXXXX` is the last three bytes of the
//! Wi-Fi MAC; clients see the same handle whether they discover us
//! over BLE or mDNS.
//!
//! Services exposed (in declaration order):
//!
//! - **Device Information (`0x180A`)** — manufacturer, model, and
//!   firmware-revision strings.
//! - **Battery (`0x180F`)** — `BATTERY_LEVEL` (`0x2A19`) reflecting
//!   `snapshot::read().battery.percent`. Periodic notify @ 1 Hz when
//!   a peer is subscribed.
//! - **Stack-chan custom service** — emotion characteristic
//!   (one-byte enum, [`stackchan_core::Emotion::wire_byte`]).
//!   Notify on transition only.
//! - **Provisioning custom service** — writeable SSID + PSK
//!   characteristics. Writing the PSK commits the staged SSID + new
//!   PSK and signals [`crate::net::wifi::WIFI_RECONFIG`].
//!
//! Pairing: LE Secure Connections with `IoCapabilities::DisplayOnly`.
//! On the first `PassKeyDisplay` event the 6-digit passkey is latched
//! into [`PASSKEY_DISPLAY`] and the render task overlays it on the
//! LCD. Bonds are persisted to the SD card via [`bonds`] so a
//! re-paired peer skips the passkey dance.
//!
//! Coexistence: BLE rides the same radio as Wi-Fi via esp-radio's
//! `coex` feature. Expect a single-digit-percent Wi-Fi throughput
//! cost — invisible for the dashboard / SSE / settings round-trips
//! the firmware does today.
//!
//! Sync-version note: trouble-host 0.5.x pulls
//! `embassy-sync = 0.6` while the rest of the firmware uses 0.7.
//! Both compile side-by-side because nothing crosses the boundary —
//! shared state goes through `snapshot::read()` (a
//! `Mutex<CriticalSectionRawMutex, Cell<...>>`), never via embassy
//! `Signal`s.

pub mod bonds;
mod server;
mod task;

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub use server::{StackchanServer, run_ble_peripheral};
pub use task::{BleTaskConfig, ble_task};

/// Latched 6-digit pairing passkey, displayed on the LCD while a
/// central is in the middle of pairing.
///
/// The render task reads it each frame; the BLE task writes it on
/// `PassKeyDisplay`, clears it on `PairingComplete` / `PairingFailed`
/// / `Disconnected`. Latched (rather than `Signal`-style one-shot)
/// because the render task draws the same value on every frame the
/// passkey is visible — `Signal::try_take` would consume it once and
/// blank the LCD after a single frame.
static PASSKEY_DISPLAY: Mutex<CriticalSectionRawMutex, Cell<Option<u32>>> =
    Mutex::new(Cell::new(None));

/// Show a 6-digit passkey on the LCD. Called by the BLE task on
/// receiving a `PassKeyDisplay` event. The next render frame picks
/// it up via [`read_passkey`] and overlays it on the avatar.
pub fn show_passkey(passkey: u32) {
    PASSKEY_DISPLAY.lock(|cell| cell.set(Some(passkey)));
}

/// Clear the on-screen passkey. Called when pairing completes
/// (success or failure) or the central disconnects mid-pairing.
pub fn clear_passkey() {
    PASSKEY_DISPLAY.lock(|cell| cell.set(None));
}

/// Read the latched passkey for the current render frame. Returns
/// `None` when no pairing is in flight.
#[must_use]
pub fn read_passkey() -> Option<u32> {
    PASSKEY_DISPLAY.lock(core::cell::Cell::get)
}
