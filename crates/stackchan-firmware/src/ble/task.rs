//! Embassy task entry point for the BLE peripheral.
//!
//! This file pins the otherwise-generic [`run_ble_peripheral`] to the
//! concrete controller type (`ExternalController<BleConnector<'static>,
//! 20>`) so it can be spawned with `embassy_executor::task`. The 20 is
//! the depth of the HCI command queue between trouble-host and the
//! esp-radio controller — matches the trouble-host esp32 example and
//! is plenty for our single-peer peripheral surface.

use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::ExternalController;

use super::server::run_ble_peripheral;

/// Boot-time inputs to the BLE task.
///
/// The controller is built in `main.rs` so it can borrow the same
/// `&'static esp_radio::Controller` the Wi-Fi side uses; the
/// leftover bookkeeping (advertised name, random address bytes) is
/// plumbed through this struct.
pub struct BleTaskConfig {
    /// Connector handed off from `BleConnector::new(&radio, BT, _)`.
    pub controller: ExternalController<BleConnector<'static>, 20>,
    /// Random LE address advertised by the device. Derived from the
    /// MAC at boot; see [`crate::ble::ble_task`]'s caller in
    /// `main.rs`.
    pub address_bytes: [u8; 6],
    /// Local name shown in scans (`stackchan-XXXXXX`). Held for
    /// the firmware lifetime via `StaticCell` in `main.rs`.
    pub local_name: &'static str,
}

/// Spawnable BLE task. Owns the controller and runs the trouble-host
/// stack forever.
#[embassy_executor::task]
pub async fn ble_task(config: BleTaskConfig) -> ! {
    let BleTaskConfig {
        controller,
        address_bytes,
        local_name,
    } = config;
    run_ble_peripheral(controller, address_bytes, local_name).await
}
