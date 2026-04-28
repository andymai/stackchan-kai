//! GATT server declaration + per-connection serve loop.
//!
//! ## Lints
//!
//! `missing_docs` is silenced for the whole module because the
//! `#[gatt_server]` and `#[gatt_service]` proc-macros emit public
//! support items (auxiliary `handle: u16` fields, `new` /
//! `new_with_config` constructors, `get` / `set` methods) without
//! doc comments. Our own struct/field doc comments below stay
//! authoritative for the human-facing surface.
//!
//! The server exposes three services beyond the auto-built GAP:
//!
//! 1. **Device Information `0x180A`** — manufacturer / model /
//!    firmware-revision strings. Populated at boot and never mutated.
//! 2. **Battery `0x180F`** — battery level (`0x2A19`), `read + notify`.
//! 3. **Stack-chan custom service** (128-bit UUID
//!    `8a1c0001-7b3f-4d52-9c6e-5f5ba1e5cf01`) — emotion characteristic
//!    (`8a1c0002-…`), `read + notify`, one byte using
//!    [`stackchan_core::Emotion::wire_byte`].
//!
//! DIS characteristic types are `heapless::String<N>` rather than
//! `&'static str` because trouble-host's `AsGatt for &'static str`
//! sets `MAX_SIZE = usize::MAX`, which the gatt-service macro tries
//! to allocate as a static byte buffer — it doesn't fit.
//! `heapless::String<N>` caps the storage at N bytes and is set at
//! runtime via [`StackchanServer::set`].

#![allow(missing_docs)]

use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
// trouble-host 0.5 implements `AsGatt` on `heapless::String<N>` from
// the 0.9 line; the firmware's other uses of `heapless` still ride
// 0.8. Reach explicitly for the alias so the right impl is picked.
use heapless_09::String as HString;
use stackchan_core::Emotion;
use trouble_host::Address;
use trouble_host::prelude::*;

use crate::net::snapshot;

/// Manufacturer string for DIS. Stack-chan rides the M5Stack CoreS3.
const DIS_MANUFACTURER: &str = "M5Stack";

/// Model string for DIS — the human-readable product identity.
const DIS_MODEL: &str = "Stack-chan CoreS3";

/// Firmware revision shown over BLE. Wired from cargo metadata so the
/// value tracks release-please bumps without manual sync.
const DIS_FIRMWARE_REVISION: &str = env!("CARGO_PKG_VERSION");

/// Storage cap for each DIS string characteristic. 32 bytes is more
/// than enough for `M5Stack`, `Stack-chan CoreS3`, and any
/// foreseeable cargo version string.
const DIS_FIELD_CAP: usize = 32;

/// Maximum simultaneous BLE centrals. One phone at a time is plenty.
const CONNECTIONS_MAX: usize = 1;

/// L2CAP channels: ATT + signaling. We don't open L2CAP `CoC`, so two
/// is enough.
const L2CAP_CHANNELS_MAX: usize = 2;

/// Notify cadence for the battery characteristic. Battery state moves
/// slowly; 1 Hz is generous and stays well below the BLE radio's
/// busy threshold.
const BATTERY_NOTIFY_PERIOD: Duration = Duration::from_secs(1);

/// Top-level GATT server. The `#[gatt_server]` macro emits a struct
/// with a `'values` lifetime parameter that ties the GAP name + any
/// `&'static str` characteristic defaults to a single anchor.
///
/// The `#[allow(missing_docs)]` covers macro-generated internals
/// (the auxiliary `handle` fields + `new` methods that the
/// `gatt_server` / `gatt_service` macros emit alongside our own
/// fields) — those are public so the macro-emitted constructors can
/// see them, but they're not part of our intentional surface.
#[allow(missing_docs)]
#[gatt_server]
pub struct StackchanServer {
    /// Device Information Service (`0x180A`) — manufacturer / model /
    /// firmware-revision strings, populated at boot.
    pub dis: DeviceInformationService,
    /// Battery Service (`0x180F`) — battery level percent, notified
    /// at 1 Hz when subscribed.
    pub battery: BatteryService,
    /// Stack-chan custom service — emotion characteristic, notified
    /// on transition.
    pub stackchan: StackchanService,
}

#[allow(missing_docs)]
#[gatt_service(uuid = service::DEVICE_INFORMATION)]
pub struct DeviceInformationService {
    #[characteristic(uuid = characteristic::MANUFACTURER_NAME_STRING, read)]
    pub manufacturer: HString<DIS_FIELD_CAP>,
    #[characteristic(uuid = characteristic::MODEL_NUMBER_STRING, read)]
    pub model: HString<DIS_FIELD_CAP>,
    #[characteristic(uuid = characteristic::FIRMWARE_REVISION_STRING, read)]
    pub firmware_revision: HString<DIS_FIELD_CAP>,
}

#[allow(missing_docs)]
#[gatt_service(uuid = service::BATTERY)]
pub struct BatteryService {
    /// Battery state of charge, percent 0..=100.
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 0)]
    pub level: u8,
}

#[allow(missing_docs)]
#[gatt_service(uuid = "8a1c0001-7b3f-4d52-9c6e-5f5ba1e5cf01")]
pub struct StackchanService {
    /// Current emotion, encoded by [`Emotion::wire_byte`]. Notified on
    /// transition.
    #[characteristic(uuid = "8a1c0002-7b3f-4d52-9c6e-5f5ba1e5cf01", read, notify, value = 0)]
    pub emotion: u8,
}

/// Run the BLE peripheral for the firmware lifetime.
///
/// Owns the trouble-host stack and loops `advertise → accept → serve`.
/// Returns only on unrecoverable controller failure (in which case the
/// caller should park rather than restart, since esp-radio's BLE
/// transport can't be safely re-initialized while the controller is
/// still alive).
pub async fn run_ble_peripheral<C: Controller>(
    controller: C,
    address_bytes: [u8; 6],
    local_name: &'static str,
) -> ! {
    // BLE static random address requires bits 47:46 == 0b11 (Core
    // Spec Vol 6, Part B, §1.3.2.1). `Address::random` doesn't
    // enforce this, and a typical ESP32 OUI starts with `0x24`
    // which clears those bits — the controller silently rejects
    // the address on most units. `bt-hci` stores the address
    // little-endian, so the MSB lives at index 5.
    let mut address_bytes = address_bytes;
    address_bytes[5] |= 0xC0;
    let address = Address::random(address_bytes);
    defmt::info!(
        "ble: address={=[u8; 6]:02x} name={=str}",
        address_bytes,
        local_name
    );

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    let server = match StackchanServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: local_name,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    })) {
        Ok(s) => s,
        Err(e) => defmt::panic!("ble: server build failed ({=str})", e),
    };

    populate_dis(&server);

    let advertise_serve = async {
        loop {
            match advertise(local_name, &mut peripheral, &server).await {
                Ok(conn) => {
                    defmt::info!("ble: peer connected");
                    let events = gatt_events_task(&conn);
                    let notify = notify_task(&server, &conn);
                    let _ = select(events, notify).await;
                    defmt::info!("ble: peer disconnected");
                }
                Err(e) => {
                    defmt::warn!("ble: advertise failed ({})", defmt::Debug2Format(&e));
                    Timer::after(Duration::from_millis(500)).await;
                }
            }
        }
    };

    join(ble_runner(runner), advertise_serve).await;
    defmt::panic!("ble: stack exited (unrecoverable)");
}

/// Populate the static DIS characteristic values at boot. Failures
/// here are degraded-but-not-fatal: a phone reading DIS would see an
/// empty string instead of the friendly value, but the device still
/// advertises and the battery / emotion services remain usable.
fn populate_dis(server: &StackchanServer<'_>) {
    let mut s: HString<DIS_FIELD_CAP> = HString::new();
    if s.push_str(DIS_MANUFACTURER).is_err() {
        defmt::warn!("ble: manufacturer string overflow");
    }
    if let Err(e) = server.set(&server.dis.manufacturer, &s) {
        defmt::warn!("ble: dis manufacturer set ({})", defmt::Debug2Format(&e));
    }

    s.clear();
    if s.push_str(DIS_MODEL).is_err() {
        defmt::warn!("ble: model string overflow");
    }
    if let Err(e) = server.set(&server.dis.model, &s) {
        defmt::warn!("ble: dis model set ({})", defmt::Debug2Format(&e));
    }

    s.clear();
    if s.push_str(DIS_FIRMWARE_REVISION).is_err() {
        defmt::warn!("ble: firmware-revision string overflow");
    }
    if let Err(e) = server.set(&server.dis.firmware_revision, &s) {
        defmt::warn!(
            "ble: dis firmware-revision set ({})",
            defmt::Debug2Format(&e)
        );
    }
}

/// Background task: drives the trouble-host runner. Required to stay
/// alive alongside any other BLE task — the runner pumps HCI events
/// and timer ticks for the host stack.
async fn ble_runner<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            defmt::warn!("ble: runner error ({})", defmt::Debug2Format(&e));
            // Brief pause to avoid spinning on a transient HCI error.
            Timer::after(Duration::from_millis(500)).await;
        }
    }
}

/// Advertise as `<local_name>` with a connectable, scannable, undirected
/// payload. Includes the Battery service UUID in the AD so iOS battery
/// widgets can find us at scan-time.
async fn advertise<'values, 'server, C: Controller>(
    local_name: &str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server StackchanServer<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv_data = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]), // 0x180F battery service, little-endian.
            AdStructure::CompleteLocalName(local_name.as_bytes()),
        ],
        &mut adv_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &AdvertisementParameters::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    Ok(conn)
}

/// Drains GATT events for one connection. trouble-host requires every
/// event to be `accept()`ed — the reply path runs the ATT response
/// back to the central. Without this loop, reads would silently time
/// out from the peer's view.
async fn gatt_events_task<P: PacketPool>(conn: &GattConnection<'_, '_, P>) {
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                defmt::info!("ble: gatt disconnect ({})", defmt::Debug2Format(&reason));
                return;
            }
            GattConnectionEvent::Gatt { event } => match event.accept() {
                Ok(reply) => reply.send().await,
                Err(e) => defmt::warn!(
                    "ble: gatt event accept failed ({})",
                    defmt::Debug2Format(&e)
                ),
            },
            _ => {}
        }
    }
}

/// Periodic battery notify (1 Hz) + change-driven emotion notify.
///
/// Reads the snapshot every tick. Battery percent always notifies so
/// first-time subscribers see a value within a second. Emotion notifies
/// only on transition — the value barely changes at steady state, so
/// notifying every tick would waste airtime and flicker in nRF Connect.
async fn notify_task<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let battery_handle = server.battery.level;
    let emotion_handle = server.stackchan.emotion;
    let mut last_emotion: Option<Emotion> = None;

    loop {
        let snap = snapshot::read();
        let battery_pct = snap.battery.percent.unwrap_or(0);

        if let Err(e) = server.set(&battery_handle, &battery_pct) {
            defmt::warn!(
                "ble: battery table set failed ({})",
                defmt::Debug2Format(&e)
            );
        }
        // Notify failures here cover two cases: (1) the central
        // hasn't subscribed via CCCD yet (common in the first few
        // seconds after connect; trouble-host returns an error
        // because there's no notify destination), and (2) the
        // connection actually dropped. We don't try to distinguish:
        // a real disconnect surfaces in `gatt_events_task`'s
        // `Disconnected` arm, which terminates the outer
        // `select(events, notify)`. Returning here on every error
        // would race that path and end the connection on the first
        // pre-subscription tick — the Greptile P2.
        if let Err(e) = battery_handle.notify(conn, &battery_pct).await {
            defmt::trace!(
                "ble: battery notify skipped ({}) — peer not subscribed?",
                defmt::Debug2Format(&e)
            );
        }

        let emotion_byte = snap.emotion.wire_byte();
        if let Err(e) = server.set(&emotion_handle, &emotion_byte) {
            defmt::warn!(
                "ble: emotion table set failed ({})",
                defmt::Debug2Format(&e)
            );
        }
        if last_emotion != Some(snap.emotion) {
            last_emotion = Some(snap.emotion);
            if let Err(e) = emotion_handle.notify(conn, &emotion_byte).await {
                defmt::trace!(
                    "ble: emotion notify skipped ({}) — peer not subscribed?",
                    defmt::Debug2Format(&e)
                );
            }
        }

        Timer::after(BATTERY_NOTIFY_PERIOD).await;
    }
}
