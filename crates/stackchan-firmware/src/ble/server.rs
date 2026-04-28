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
//! The server exposes four services beyond the auto-built GAP:
//!
//! 1. **Device Information `0x180A`** — manufacturer / model /
//!    firmware-revision strings. Populated at boot and never mutated.
//! 2. **Battery `0x180F`** — battery level (`0x2A19`), `read + notify`.
//! 3. **Stack-chan custom service** (128-bit UUID
//!    `8a1c0001-7b3f-4d52-9c6e-5f5ba1e5cf01`) — emotion characteristic
//!    (`8a1c0002-…`), `read + notify`, one byte using
//!    [`stackchan_core::Emotion::wire_byte`].
//! 4. **Provisioning custom service** (`8a1c0010-…`) — writeable SSID
//!    + PSK characteristics. Writing the PSK commits the staged SSID
//!    + new PSK through the same atomic SD-writeback path that `PUT
//!      /settings` uses, then signals
//!      [`crate::net::wifi::WIFI_RECONFIG`] so the wifi task soft-
//!      reconnects without a reboot.
//!
//! ## Security
//!
//! The provisioning service is **unauthenticated** in this PR — any
//! BLE central in radio range can write SSID/PSK and reconfigure the
//! device. PR3 adds passkey display + LE Secure Connections bonding
//! and gates these writes to bonded peers only. Until then, the
//! `defmt::warn!` on every PSK commit is the loud reminder that this
//! is provisional.
//!
//! DIS characteristic types are `heapless::String<N>` rather than
//! `&'static str` because trouble-host's `AsGatt for &'static str`
//! sets `MAX_SIZE = usize::MAX`, which the gatt-service macro tries
//! to allocate as a static byte buffer — it doesn't fit.
//! `heapless::String<N>` caps the storage at N bytes and is set at
//! runtime via [`StackchanServer::set`].

#![allow(missing_docs)]

use alloc::string::String as AString;
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
use crate::net::wifi::{WIFI_RECONFIG, WifiCreds};
use crate::storage::{CONFIG_SNAPSHOT, with_storage};

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

/// SSID storage cap. The 802.11 spec caps SSIDs at 32 bytes.
const PROV_SSID_CAP: usize = 32;

/// PSK storage cap. WPA2-PSK ASCII is 8–63 chars; 64 bytes leaves a
/// byte of slack for an opaque trailing byte.
const PROV_PSK_CAP: usize = 64;

/// Minimum WPA2-PSK length in characters. The 802.11 spec rejects
/// passphrases shorter than this — committing one would just stick
/// the wifi task in a retry-backoff loop.
const PROV_PSK_MIN: usize = 8;

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
    /// Wi-Fi provisioning service — writeable SSID + PSK. Commits +
    /// soft-reconnects on PSK write.
    pub provisioning: ProvisioningService,
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

#[allow(missing_docs)]
#[gatt_service(uuid = "8a1c0010-7b3f-4d52-9c6e-5f5ba1e5cf01")]
pub struct ProvisioningService {
    /// Staged SSID. `read + write`: a phone app can pre-fill the
    /// current value, replace it, and the new bytes are held in the
    /// GATT table until the PSK characteristic write triggers commit.
    /// 32-byte cap matches the 802.11 SSID limit.
    #[characteristic(uuid = "8a1c0011-7b3f-4d52-9c6e-5f5ba1e5cf01", read, write)]
    pub ssid: HString<PROV_SSID_CAP>,
    /// Pre-shared key. `write`-only — read access would let any
    /// scanner exfiltrate the network secret. Writing this character-
    /// istic triggers commit: the staged SSID + new PSK are persisted
    /// to `STACKCHAN.RON`, then `WIFI_RECONFIG` is signaled so the
    /// wifi task soft-reconnects without a reboot.
    #[characteristic(uuid = "8a1c0012-7b3f-4d52-9c6e-5f5ba1e5cf01", write)]
    pub psk: HString<PROV_PSK_CAP>,
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
    populate_provisioning_ssid(&server).await;

    let advertise_serve = async {
        loop {
            match advertise(local_name, &mut peripheral, &server).await {
                Ok(conn) => {
                    defmt::info!("ble: peer connected");
                    let events = gatt_events_task(&server, &conn);
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
///
/// On a write to the provisioning PSK characteristic, fires the
/// commit path — the staged SSID + new PSK get persisted to
/// `STACKCHAN.RON` and the wifi task gets nudged to soft-reconnect.
async fn gatt_events_task<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let psk_handle = server.provisioning.psk.handle;
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                defmt::info!("ble: gatt disconnect ({})", defmt::Debug2Format(&reason));
                return;
            }
            GattConnectionEvent::Gatt { event } => {
                let is_psk_write =
                    matches!(&event, GattEvent::Write(w) if w.handle() == psk_handle);
                // Only fire commit when the write was actually
                // applied to the GATT table. A failed `accept()`
                // means trouble-host did not store the new bytes,
                // and `server.get(&psk)` would return stale or
                // empty data — committing on that path would
                // persist the wrong PSK (or treat an empty PSK as
                // an open-AP credential).
                let accepted_ok = match event.accept() {
                    Ok(reply) => {
                        reply.send().await;
                        true
                    }
                    Err(e) => {
                        defmt::warn!(
                            "ble: gatt event accept failed ({})",
                            defmt::Debug2Format(&e)
                        );
                        false
                    }
                };
                if is_psk_write && accepted_ok {
                    commit_provisioning(server).await;
                }
            }
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

/// Pre-fill the SSID characteristic value with the currently-persisted
/// network name so a phone reading it sees the active config rather
/// than an empty string. The PSK is intentionally not pre-filled —
/// the characteristic is write-only.
async fn populate_provisioning_ssid(server: &StackchanServer<'_>) {
    let snap_ssid = match CONFIG_SNAPSHOT.lock().await.as_ref() {
        Some(cfg) => cfg.wifi.ssid.clone(),
        None => return,
    };
    let mut s: HString<PROV_SSID_CAP> = HString::new();
    if s.push_str(snap_ssid.as_str()).is_err() {
        defmt::warn!(
            "ble: persisted SSID exceeds {=usize} bytes; not pre-filling",
            PROV_SSID_CAP
        );
        return;
    }
    if let Err(e) = server.set(&server.provisioning.ssid, &s) {
        defmt::warn!("ble: provisioning SSID set ({})", defmt::Debug2Format(&e));
    }
}

/// Commit a provisioning write: take the staged SSID + just-written
/// PSK from the GATT table, validate, persist atomically, signal the
/// wifi task to soft-reconnect, then clear the PSK from the table so
/// a memory dump of the BLE stack doesn't leak the secret.
///
/// Failures are logged but never panic — a malformed write should
/// just be a no-op so the next correct write can succeed.
async fn commit_provisioning(server: &StackchanServer<'_>) {
    defmt::warn!(
        "ble: provisioning commit (UNAUTHENTICATED — pairing pending). \
         Anyone in BLE range can reconfigure Wi-Fi until that lands."
    );

    // Read staged SSID + new PSK from the GATT table. trouble-host
    // wrote both into the table when their respective Write events
    // were accepted.
    let staged_ssid: HString<PROV_SSID_CAP> = match server.get(&server.provisioning.ssid) {
        Ok(v) => v,
        Err(e) => {
            defmt::warn!("ble: prov: ssid get failed ({})", defmt::Debug2Format(&e));
            return;
        }
    };
    let new_psk: HString<PROV_PSK_CAP> = match server.get(&server.provisioning.psk) {
        Ok(v) => v,
        Err(e) => {
            defmt::warn!("ble: prov: psk get failed ({})", defmt::Debug2Format(&e));
            return;
        }
    };

    if staged_ssid.trim().is_empty() {
        defmt::warn!("ble: prov: empty SSID — write SSID before PSK to commit");
        return;
    }
    // Reject below-spec PSKs at the BLE boundary so they never reach
    // the SD card or the wifi task. Committing a 1–7 char PSK would
    // just thrash the radio on a retry loop. Empty PSK is allowed
    // for provisioning an open AP (which the 802.11 driver will
    // accept).
    if !new_psk.is_empty() && new_psk.len() < PROV_PSK_MIN {
        defmt::warn!(
            "ble: prov: PSK too short ({=usize} bytes; need {=usize}+)",
            new_psk.len(),
            PROV_PSK_MIN
        );
        return;
    }

    // Build a new Config from the persisted snapshot, mutating only
    // the wifi block. Any other settings (mDNS hostname, SNTP, audio)
    // stay untouched, mirroring the conservative-update shape of
    // PUT /settings's merge step.
    //
    // Snapshot is cloned out-of-line so the mutex guard drops before
    // the subsequent `with_storage` call (also under a mutex) — both
    // mutexes touch the SD via the boot path, so holding both at once
    // would risk a re-entrant deadlock if storage internals ever grew
    // a CONFIG_SNAPSHOT touch.
    let snapshot_value = CONFIG_SNAPSHOT.lock().await.clone();
    let Some(mut new_config) = snapshot_value else {
        defmt::warn!("ble: prov: no config snapshot — boot config not yet read");
        return;
    };
    new_config.wifi.ssid = AString::from(staged_ssid.as_str());
    new_config.wifi.psk = AString::from(new_psk.as_str());

    let write_result = with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!(
                "ble: prov: persisted (ssid={=str})",
                new_config.wifi.ssid.as_str()
            );
            *CONFIG_SNAPSHOT.lock().await = Some(new_config.clone());
            WIFI_RECONFIG.signal(WifiCreds {
                ssid: new_config.wifi.ssid,
                psk: new_config.wifi.psk,
            });
        }
        Some(Err(e)) => {
            defmt::warn!("ble: prov: write_config failed ({})", e);
        }
        None => {
            defmt::warn!("ble: prov: no SD mounted — cannot persist");
        }
    }

    // Clear the PSK from the GATT table. The central just wrote it
    // and could theoretically read it back if `read` were enabled
    // (it isn't), but a future maintenance change to the macro
    // attributes shouldn't accidentally start leaking secrets.
    let empty: HString<PROV_PSK_CAP> = HString::new();
    if let Err(e) = server.set(&server.provisioning.psk, &empty) {
        defmt::warn!(
            "ble: prov: psk-clear table set ({})",
            defmt::Debug2Format(&e)
        );
    }
}
