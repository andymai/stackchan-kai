//! BLE peripheral surface — advertise + read-only GATT.
//!
//! On boot, [`ble_task`] takes ownership of the trouble-host
//! `ExternalController` (built from `esp_radio`'s `BleConnector`) and
//! drives an advertise → accept-connection → serve-GATT loop for the
//! firmware lifetime. The advertised local name is
//! `stackchan-XXXXXX`, where `XXXXXX` is the last three bytes of the
//! Wi-Fi MAC; clients see the same handle whether they discover us
//! over BLE or mDNS.
//!
//! Three services are exposed in this initial cut, all read-only:
//!
//! - **Device Information (`0x180A`)** — manufacturer, model, and
//!   firmware-revision strings. nRF Connect surfaces these natively.
//! - **Battery (`0x180F`)** — `BATTERY_LEVEL` (`0x2A19`) reflecting
//!   `snapshot::read().battery.percent`. Periodic notify @ 1 Hz when
//!   a peer is subscribed.
//! - **Stack-chan custom service** — emotion characteristic
//!   (one-byte enum, [`stackchan_core::Emotion::wire_byte`]).
//!   Notify on transition only.
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

mod server;
mod task;

pub use server::{StackchanServer, run_ble_peripheral};
pub use task::{BleTaskConfig, ble_task};
