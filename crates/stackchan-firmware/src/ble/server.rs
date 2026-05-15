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
//! The server exposes the following services beyond the auto-built GAP:
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
//! 5. **Audio service** (`8a1c0020-…`) — volume + mute, both
//!    `read + write + notify`. Writes route through
//!    [`crate::audio::persist_volume`] / [`crate::audio::persist_mute`]
//!    so HTTP `POST /volume` / `POST /mute` and the BLE write share
//!    one persistence implementation.
//! 6. **Avatar control service** (`8a1c0030-…`) — `write`-only
//!    characteristics for emotion, look-at, reset, and speak. Each
//!    write decodes a fixed-length payload via
//!    [`stackchan_net::ble_command`] and signals
//!    [`crate::net::http::REMOTE_COMMAND_SIGNAL`], the same channel
//!    the HTTP control plane drives.
//! 7. **View service** (`8a1c0040-…`) — display + capture
//!    triggers. Camera-mode flag (`8a1c0041-…`, `read + write +
//!    notify`) mirrors HTTP `POST /camera/mode`; camera-capture
//!    (`8a1c0042-…`, `write`-only, any 1-byte trigger) mirrors HTTP
//!    `POST /camera/capture` and signals the camera task to persist
//!    the latest frame to `/sd/CAPTURE.565`.
//! 8. **`BluFi` service** (16-bit UUID `0xFFFF`) — Espressif's BLE
//!    Wi-Fi provisioning protocol. Write characteristic (`0xFF01`)
//!    receives `BluFi` frames parsed via
//!    [`stackchan_net::blufi::parse_frame`]; notify characteristic
//!    (`0xFF02`) is reserved for outbound status frames. The current
//!    slice surfaces parsed frames in defmt logs only — the
//!    SSID/password accumulator and the `ControlSubtype::ConnectToAp`
//!    commit path land in a follow-up.
//!
//! ## Security
//!
//! Control writes (audio + avatar services) and provisioning writes
//! both require an `EncryptedAuthenticated` link (passkey-confirmed
//! bond). Centrals that haven't paired get an
//! `INSUFFICIENT_AUTHENTICATION` reject for control writes; the
//! provisioning path additionally validates payload contents at
//! commit time and silently no-ops on auth failure (legacy from the
//! initial provisioning PR).
//!
//! **`BluFi` exception**: writes to the `BluFiService` write
//! characteristic are *not* auth-gated at the ATT layer. The standard
//! ESP BLE Provisioning Android / iOS app sends `BluFi` frames before
//! the pairing handshake, and `BluFi`'s own `SetSecMode` + AES-CCM
//! (not yet implemented) is what protects payloads on the wire.
//! The `ControlSubtype::ConnectToAp` commit path re-uses the same
//! `EncryptedAuthenticated` gate as the custom provisioning service
//! (via the shared [`apply_wifi_credentials`] helper), so the
//! security posture at the *commit* boundary matches: a central
//! that hasn't authenticated can stage credentials in-memory in
//! its per-connection [`BluFiSession`] but cannot persist them to
//! SD or trigger a Wi-Fi reconfiguration.
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
// trouble-host 0.5 implements `AsGatt` on `heapless::String<N>` /
// `heapless::Vec<u8, _>` from the 0.9 line; the firmware's other uses
// of `heapless` still ride 0.8. Reach explicitly for the aliases so
// the right impls are picked.
use heapless_09::String as HString;
use heapless_09::Vec as HVec;
use stackchan_core::Emotion;
use stackchan_net::ble_command::{self, BleError, EMOTION_WRITE_LEN, LOOK_AT_LEN, SPEAK_LEN};
use stackchan_net::blufi;
use trouble_host::Address;
use trouble_host::prelude::*;

use crate::audio::AudioPersistOutcome;
use crate::camera::CAMERA_MODE_SIGNAL;
use crate::net::http::REMOTE_COMMAND_SIGNAL;
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

/// Buffer cap for the `BluFi` inbound-frame characteristic. The BLE LE
/// LL PDU caps at 251 bytes; subtract the ATT-write header (3 bytes)
/// and a byte of slack, leaving 247 — the practical largest payload
/// a central can deliver in a single ATT operation. `BluFi` frame
/// fragmentation rides the protocol-level `FC_FRAGMENT` bit, not GATT-
/// layer chunking, so one ATT write maps to exactly one frame.
const BLUFI_FRAME_BUF: usize = 247;

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
    /// Audio service — volume + mute (read + write + notify). Mirrors
    /// HTTP `POST /volume` / `POST /mute`.
    pub audio: AudioService,
    /// Avatar control service — emotion / look-at / reset / speak
    /// (write only). Mirrors HTTP `POST /emotion`, `/look-at`,
    /// `/reset`, `/speak`.
    pub avatar: AvatarControlService,
    /// View service — display-related toggles. Currently exposes
    /// camera-mode (read + write + notify); future entries may add
    /// LCD brightness / render mode. Mirrors HTTP `POST /camera/mode`.
    pub view: ViewService,
    /// `BluFi` service — Espressif's BLE Wi-Fi provisioning protocol.
    /// Inbound frames land on the write characteristic; the notify
    /// characteristic is reserved for outbound status frames in a
    /// follow-up slice.
    pub blufi: BluFiService,
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
#[gatt_service(uuid = "8a1c0020-7b3f-4d52-9c6e-5f5ba1e5cf01")]
pub struct AudioService {
    /// Volume percentile (`0..=100`). `read + write + notify`.
    /// Decoded via [`stackchan_net::ble_command::decode_volume`];
    /// writes route through [`crate::audio::persist_volume`].
    #[characteristic(
        uuid = "8a1c0021-7b3f-4d52-9c6e-5f5ba1e5cf01",
        read,
        write,
        notify,
        value = 0
    )]
    pub volume: u8,
    /// Mute flag (`0 = un-muted`, `1 = muted`). `read + write + notify`.
    /// Decoded via [`stackchan_net::ble_command::decode_mute`];
    /// writes route through [`crate::audio::persist_mute`].
    #[characteristic(
        uuid = "8a1c0022-7b3f-4d52-9c6e-5f5ba1e5cf01",
        read,
        write,
        notify,
        value = 0
    )]
    pub mute: u8,
}

#[allow(missing_docs)]
#[gatt_service(uuid = "8a1c0030-7b3f-4d52-9c6e-5f5ba1e5cf01")]
pub struct AvatarControlService {
    /// Emotion override. 3-byte payload (`u8` emotion + `u16 LE`
    /// `hold_ms`). See [`stackchan_net::ble_command::decode_emotion_write`].
    #[characteristic(uuid = "8a1c0031-7b3f-4d52-9c6e-5f5ba1e5cf01", write, value = [0u8; EMOTION_WRITE_LEN])]
    pub emotion_write: [u8; EMOTION_WRITE_LEN],
    /// Look-at override. 6-byte payload (two `i16 LE` centi-degrees +
    /// `u16 LE` `hold_ms`). See [`stackchan_net::ble_command::decode_look_at`].
    #[characteristic(uuid = "8a1c0032-7b3f-4d52-9c6e-5f5ba1e5cf01", write, value = [0u8; LOOK_AT_LEN])]
    pub look_at: [u8; LOOK_AT_LEN],
    /// Reset trigger. Any 1-byte write clears active emotion / look-at
    /// holds. See [`stackchan_net::ble_command::decode_reset`].
    #[characteristic(uuid = "8a1c0033-7b3f-4d52-9c6e-5f5ba1e5cf01", write, value = 0)]
    pub reset: u8,
    /// Speak trigger. 2-byte payload (`u8` phrase + `u8` locale). See
    /// [`stackchan_net::ble_command::decode_speak`].
    #[characteristic(uuid = "8a1c0034-7b3f-4d52-9c6e-5f5ba1e5cf01", write, value = [0u8; SPEAK_LEN])]
    pub speak: [u8; SPEAK_LEN],
}

#[allow(missing_docs)]
#[gatt_service(uuid = "8a1c0040-7b3f-4d52-9c6e-5f5ba1e5cf01")]
pub struct ViewService {
    /// LCD display-mode toggle. `0 = avatar`, `1 = camera preview`.
    /// `read + write + notify`. Decoded via
    /// [`stackchan_net::ble_command::decode_camera_mode`]; writes
    /// route through [`crate::net::snapshot::update_camera_mode`] +
    /// [`crate::camera::CAMERA_MODE_SIGNAL`].
    #[characteristic(
        uuid = "8a1c0041-7b3f-4d52-9c6e-5f5ba1e5cf01",
        read,
        write,
        notify,
        value = 0
    )]
    pub camera_mode: u8,
    /// Camera-capture trigger. `write`-only — any 1-byte write
    /// signals the camera task to write the latest QVGA RGB565 frame
    /// to `/sd/CAPTURE.565`. Decoded via
    /// [`stackchan_net::ble_command::decode_camera_capture`].
    #[characteristic(uuid = "8a1c0042-7b3f-4d52-9c6e-5f5ba1e5cf01", write, value = 0)]
    pub camera_capture: u8,
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

/// `BluFi` GATT service.
///
/// The service / characteristic UUIDs are the 128-bit canonical
/// expansions of `BluFi`'s 16-bit UUIDs (`0xFFFF` / `0xFF01` /
/// `0xFF02`) via the Bluetooth Base UUID (`xxxxxxxx-0000-1000-8000-
/// 00805F9B34FB`). Centrals scanning by 16-bit UUID still match.
#[allow(missing_docs)]
#[gatt_service(uuid = "0000ffff-0000-1000-8000-00805f9b34fb")]
pub struct BluFiService {
    /// Inbound frame characteristic. Each ATT write carries exactly
    /// one `BluFi` frame; multi-frame fragmentation rides the protocol-
    /// level `FC_FRAGMENT` bit. Stored as `heapless_09::Vec<u8, …>`
    /// so the GATT layer accepts variable-length writes up to the
    /// buffer cap.
    #[characteristic(uuid = "0000ff01-0000-1000-8000-00805f9b34fb", write)]
    pub frame_write: HVec<u8, BLUFI_FRAME_BUF>,
    /// Outbound notify characteristic. Reserved for status frames
    /// (`ReportWifiStatus`, etc.) in a follow-up slice; currently
    /// declared so the service descriptor table is stable from the
    /// central's view even before notifications are wired up.
    #[characteristic(uuid = "0000ff02-0000-1000-8000-00805f9b34fb", read, notify)]
    pub frame_notify: HVec<u8, BLUFI_FRAME_BUF>,
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

    // SMP needs ~32 bytes of cryptographic entropy. The chip's TRNG
    // gives us that. `try_new` requires the global `TrngSource` to be
    // active (set up in main.rs at boot); on failure we panic — the
    // BLE peripheral is not safe to bring up without a real entropy
    // source for ECDH key generation.
    let mut trng = match esp_hal::rng::Trng::try_new() {
        Ok(t) => t,
        Err(e) => defmt::panic!(
            "ble: TRNG unavailable ({}) — TrngSource not initialised",
            defmt::Debug2Format(&e)
        ),
    };

    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_random_generator_seed(&mut trng)
        .set_io_capabilities(IoCapabilities::DisplayOnly);

    // Re-register persisted bonds before any central can start
    // pairing — a freshly-rebooted device should resume an existing
    // bond rather than asking for a re-pair on every reconnect.
    for bond in super::bonds::load_all().await {
        if let Err(e) = stack.add_bond_information(bond) {
            defmt::warn!(
                "ble: bonds: add_bond_information rejected ({})",
                defmt::Debug2Format(&e)
            );
        }
    }
    defmt::info!(
        "ble: bonds: {=usize} persisted bond(s) registered",
        stack.get_bond_information().len()
    );

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
    populate_read_state(&server);

    let advertise_serve = async {
        loop {
            match advertise(local_name, &mut peripheral, &server).await {
                Ok(conn) => {
                    defmt::info!("ble: peer connected");
                    let events = gatt_events_task(&stack, &server, &conn);
                    let notify = notify_task(&server, &conn);
                    let _ = select(events, notify).await;
                    // `gatt_events_task`'s `Disconnected` arm is the
                    // only place that calls `clear_passkey()` — but
                    // `notify_task` can win the select on an error
                    // path and drop `gatt_events_task` mid-pairing,
                    // leaving the 6-digit code painted on the LCD
                    // forever. Clear here as a belt-and-braces safety
                    // net regardless of which branch returned.
                    super::clear_passkey();
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

/// Cached write-characteristic handles, looked up once per connection
/// so the per-event dispatch is a handful of integer compares.
struct WriteHandles {
    /// Provisioning PSK write — kept here so the dispatch routes
    /// straight into [`commit_provisioning`] without a separate
    /// pattern match.
    psk: u16,
    /// Audio volume write.
    volume: u16,
    /// Audio mute write.
    mute: u16,
    /// Avatar emotion-write characteristic.
    emotion_write: u16,
    /// Avatar look-at write.
    look_at: u16,
    /// Avatar reset trigger.
    reset: u16,
    /// Avatar speak trigger.
    speak: u16,
    /// View camera-mode toggle.
    camera_mode: u16,
    /// View camera-capture trigger.
    camera_capture: u16,
    /// `BluFi` inbound-frame characteristic. Writes are parsed via
    /// [`stackchan_net::blufi::parse_frame`] and surfaced in defmt.
    blufi_frame: u16,
}

impl WriteHandles {
    /// Snapshot the write handles from the freshly-built server.
    /// Cheap — each field is one `u16` copy from a fixed offset
    /// inside the macro-emitted server struct.
    const fn new(server: &StackchanServer<'_>) -> Self {
        Self {
            psk: server.provisioning.psk.handle,
            volume: server.audio.volume.handle,
            mute: server.audio.mute.handle,
            emotion_write: server.avatar.emotion_write.handle,
            look_at: server.avatar.look_at.handle,
            reset: server.avatar.reset.handle,
            speak: server.avatar.speak.handle,
            camera_mode: server.view.camera_mode.handle,
            camera_capture: server.view.camera_capture.handle,
            blufi_frame: server.blufi.frame_write.handle,
        }
    }
}

/// Per-connection `BluFi` provisioning state.
///
/// Inbound `BluFi` `Data` frames stage SSID + password incrementally;
/// the device only persists once a `ControlSubtype::ConnectToAp`
/// control frame lands. Dropped at the end of `gatt_events_task` so
/// the staged PSK never outlives the connection.
struct BluFiSession {
    /// `SendStaSsid` payload, decoded as UTF-8. Empty until the
    /// central sends one. Overwritten on each new `SendStaSsid`.
    staged_ssid: HString<PROV_SSID_CAP>,
    /// `SendStaPassword` payload, decoded as UTF-8 (WPA2 ASCII).
    /// Cleared after successful commit so a long-lived connection
    /// doesn't hold the secret indefinitely.
    staged_psk: HString<PROV_PSK_CAP>,
    /// `SetWifiOpMode` payload byte if the central has sent one.
    /// Logged for diagnostics; we don't actually switch op-mode
    /// because the firmware only supports STA.
    op_mode: Option<u8>,
}

impl BluFiSession {
    /// Empty session — no staged credentials, no op-mode set.
    const fn new() -> Self {
        Self {
            staged_ssid: HString::new(),
            staged_psk: HString::new(),
            op_mode: None,
        }
    }
}

/// Action performed after a write has been accepted at the ATT layer.
/// Decoded from the wire payload up front so the post-accept handler
/// is a straight signal/persist call without re-parsing.
enum WriteAction {
    /// Provisioning-PSK commit (existing flow): read SSID + PSK back
    /// from the GATT table, validate, persist, signal Wi-Fi reconfig.
    ProvisioningPsk,
    /// Apply a new volume percentile via [`crate::audio::persist_volume`].
    Volume(u8),
    /// Apply a new mute flag via [`crate::audio::persist_mute`].
    Mute(bool),
    /// Forward a parsed [`stackchan_core::RemoteCommand`] onto
    /// [`REMOTE_COMMAND_SIGNAL`] — handles emotion / look-at / reset / speak.
    Remote(stackchan_core::RemoteCommand),
    /// Forward a synthesised reset onto [`REMOTE_COMMAND_SIGNAL`].
    /// Reset has no decoded payload but needs the same dispatch shape.
    RemoteReset,
    /// Apply a new camera-mode flag via
    /// [`crate::net::snapshot::update_camera_mode`] +
    /// [`CAMERA_MODE_SIGNAL`].
    CameraMode(bool),
    /// Signal the camera task to persist the latest frame to
    /// `/sd/CAPTURE.565`.
    CameraCapture,
    /// Surface a parsed `BluFi` frame in defmt. The provisioning state
    /// machine (SSID/password accumulation, commit on
    /// `ControlSubtype::ConnectToAp`) lands in a follow-up; this
    /// slice proves the GATT plumbing.
    BluFiFrame(blufi::Frame),
}

/// Decision the dispatcher takes for one Gatt event.
enum WriteDecision {
    /// Non-write event, or write to an unknown handle. Fall through
    /// to default accept-and-discard semantics.
    Default,
    /// Accept the write, then perform [`WriteAction`].
    Accept(WriteAction),
    /// Reject the write with an ATT error code. The central sees the
    /// error; the GATT table is not modified.
    Reject(AttErrorCode),
}

/// Drains GATT events for one connection. trouble-host requires every
/// event to be `accept()` (or `reject()`)-ed — the reply path runs the
/// ATT response back to the central. Without this loop, reads would
/// silently time out from the peer's view.
///
/// Also handles the BLE security event surface:
///
/// - `PassKeyDisplay` — latch the 6-digit code into [`crate::ble::PASSKEY_DISPLAY`]
///   so the render task overlays it on the LCD.
/// - `PairingComplete { bond: Some(_) }` — persist the new bond list
///   to SD via [`crate::ble::bonds`].
/// - `PairingFailed` — clear the on-screen passkey, log the reason.
///
/// Control writes (audio + avatar services) require an
/// `EncryptedAuthenticated` link; unauthenticated writes are rejected
/// with `INSUFFICIENT_AUTHENTICATION` so the GATT table never holds
/// bytes from an unpaired peer. Provisioning-PSK writes use the
/// pre-existing accept-then-commit flow; their security check lives
/// in `commit_provisioning`.
async fn gatt_events_task<P: PacketPool>(
    stack: &Stack<'_, impl Controller, P>,
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let handles = WriteHandles::new(server);
    let mut blufi_session = BluFiSession::new();
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                defmt::info!("ble: gatt disconnect ({})", defmt::Debug2Format(&reason));
                super::clear_passkey();
                return;
            }
            GattConnectionEvent::Gatt { event } => {
                dispatch_gatt_event(server, conn, &handles, &mut blufi_session, event).await;
            }
            GattConnectionEvent::PassKeyDisplay(passkey) => {
                let value = passkey.value();
                defmt::info!("ble: passkey display: {=u32:06}", value);
                super::show_passkey(value);
            }
            GattConnectionEvent::PairingComplete {
                security_level,
                bond,
            } => {
                super::clear_passkey();
                defmt::info!(
                    "ble: pairing complete (level={}, bonded={})",
                    defmt::Debug2Format(&security_level),
                    bond.is_some()
                );
                if bond.is_some() {
                    let snapshot = stack.get_bond_information();
                    super::bonds::save_all(&snapshot).await;
                }
            }
            GattConnectionEvent::PairingFailed(e) => {
                super::clear_passkey();
                defmt::warn!("ble: pairing failed ({})", defmt::Debug2Format(&e));
            }
            _ => {}
        }
    }
}

/// Dispatch one Gatt event: decide accept/reject for known writes,
/// fall through to default accept for everything else, then run any
/// post-accept action (signal + persist).
async fn dispatch_gatt_event<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
    handles: &WriteHandles,
    blufi_session: &mut BluFiSession,
    event: GattEvent<'_, '_, P>,
) {
    let decision = match &event {
        GattEvent::Write(w) => decide_write(handles, conn, w.handle(), w.data()),
        _ => WriteDecision::Default,
    };

    match decision {
        WriteDecision::Reject(err) => match event.reject(err) {
            Ok(reply) => reply.send().await,
            Err(e) => defmt::warn!("ble: gatt reject failed ({})", defmt::Debug2Format(&e)),
        },
        WriteDecision::Default => match event.accept() {
            Ok(reply) => reply.send().await,
            Err(e) => defmt::warn!("ble: gatt accept failed ({})", defmt::Debug2Format(&e)),
        },
        WriteDecision::Accept(action) => {
            // Only fire the post-accept action when the ATT reply
            // actually went out — a failed `accept()` means the
            // attribute server didn't store any new bytes, so calling
            // e.g. `commit_provisioning` against a stale GATT table
            // would persist the wrong value.
            let accepted = match event.accept() {
                Ok(reply) => {
                    reply.send().await;
                    true
                }
                Err(e) => {
                    defmt::warn!("ble: gatt accept failed ({})", defmt::Debug2Format(&e));
                    false
                }
            };
            if !accepted {
                return;
            }
            apply_write_action(server, conn, blufi_session, action).await;
        }
    }
}

/// Inspect a write event and decide whether to accept, reject, or
/// pass through. For known control writes (audio + avatar services),
/// require an authenticated link and decode the payload via
/// [`stackchan_net::ble_command`]. For everything else (including
/// the existing provisioning SSID write), keep the historical
/// accept-and-store-into-the-GATT-table behaviour.
fn decide_write<P: PacketPool>(
    handles: &WriteHandles,
    conn: &GattConnection<'_, '_, P>,
    handle: u16,
    data: &[u8],
) -> WriteDecision {
    if handle == handles.psk {
        return WriteDecision::Accept(WriteAction::ProvisioningPsk);
    }
    if handle == handles.blufi_frame {
        // `BluFi` writes are not auth-gated at the GATT layer — the
        // standard ESP BLE Provisioning app sends frames before the
        // pairing handshake, and `BluFi`'s own SetSecMode + AES-CCM
        // (out of scope this slice) is what protects the payload on
        // the wire. The commit path that lands in a follow-up slice
        // re-uses `commit_provisioning`'s authenticated-link gate at
        // the point staged credentials are persisted.
        return match blufi::parse_frame(data) {
            Ok(frame) => WriteDecision::Accept(WriteAction::BluFiFrame(frame)),
            Err(e) => {
                defmt::warn!(
                    "ble: blufi parse rejected ({}) data={=usize}B",
                    defmt::Debug2Format(&e),
                    data.len(),
                );
                WriteDecision::Reject(att_error_for_blufi(e))
            }
        };
    }
    let is_control = handle == handles.volume
        || handle == handles.mute
        || handle == handles.emotion_write
        || handle == handles.look_at
        || handle == handles.reset
        || handle == handles.speak
        || handle == handles.camera_mode
        || handle == handles.camera_capture;
    if !is_control {
        return WriteDecision::Default;
    }
    if !is_authenticated(conn) {
        return WriteDecision::Reject(AttErrorCode::INSUFFICIENT_AUTHENTICATION);
    }
    let result = if handle == handles.volume {
        ble_command::decode_volume(data).map(WriteAction::Volume)
    } else if handle == handles.mute {
        ble_command::decode_mute(data).map(WriteAction::Mute)
    } else if handle == handles.emotion_write {
        ble_command::decode_emotion_write(data).map(WriteAction::Remote)
    } else if handle == handles.look_at {
        ble_command::decode_look_at(data).map(WriteAction::Remote)
    } else if handle == handles.reset {
        ble_command::decode_reset(data).map(|()| WriteAction::RemoteReset)
    } else if handle == handles.speak {
        ble_command::decode_speak(data).map(WriteAction::Remote)
    } else if handle == handles.camera_mode {
        ble_command::decode_camera_mode(data).map(WriteAction::CameraMode)
    } else if handle == handles.camera_capture {
        ble_command::decode_camera_capture(data).map(|()| WriteAction::CameraCapture)
    } else {
        // Unreachable on the strength of the `is_control` gate above
        // — every handle counted there must have a decode arm here.
        // If a future control characteristic is added to `is_control`
        // without a matching decode branch, log loudly and reject the
        // write rather than silently mishandling it. (`debug_assert!`
        // is the wrong tool: it compiles out in release.)
        defmt::error!(
            "ble: control write dispatch missing decode arm (handle={=u16:04x})",
            handle
        );
        return WriteDecision::Reject(AttErrorCode::UNLIKELY_ERROR);
    };
    match result {
        Ok(action) => WriteDecision::Accept(action),
        Err(e) => {
            defmt::warn!(
                "ble: control write rejected (handle={=u16:04x}, {})",
                handle,
                defmt::Debug2Format(&e)
            );
            WriteDecision::Reject(att_error_for(&e))
        }
    }
}

/// Map a [`BleError`] from the wire-format codec to the ATT error
/// code the central sees. Bad length → `INVALID_ATTRIBUTE_VALUE_LENGTH`,
/// numeric range overruns → `OUT_OF_RANGE`, unknown enum bytes →
/// `VALUE_NOT_ALLOWED`.
const fn att_error_for(err: &BleError) -> AttErrorCode {
    match err {
        BleError::BadLength { .. } => AttErrorCode::INVALID_ATTRIBUTE_VALUE_LENGTH,
        BleError::VolumeOutOfRange(_) | BleError::LookAtOutOfRange { .. } => {
            AttErrorCode::OUT_OF_RANGE
        }
        BleError::BadMuteByte(_)
        | BleError::BadCameraModeByte(_)
        | BleError::UnknownEmotion(_)
        | BleError::UnknownLocale(_)
        | BleError::UnknownPhrase(_) => AttErrorCode::VALUE_NOT_ALLOWED,
    }
}

/// Map a [`blufi::ParseError`] to the ATT error code returned to the
/// central. Truncation / bad length → `INVALID_ATTRIBUTE_VALUE_LENGTH`;
/// CRC mismatch and unknown type-bits → `VALUE_NOT_ALLOWED`, since
/// the wire length itself was acceptable but the bytes are malformed.
const fn att_error_for_blufi(err: blufi::ParseError) -> AttErrorCode {
    match err {
        blufi::ParseError::Truncated | blufi::ParseError::BadDataLength => {
            AttErrorCode::INVALID_ATTRIBUTE_VALUE_LENGTH
        }
        blufi::ParseError::BadCrc | blufi::ParseError::UnknownType => {
            AttErrorCode::VALUE_NOT_ALLOWED
        }
    }
}

/// Run the post-accept side of a control write — signal the relevant
/// firmware sink and (for audio writes) persist to SD.
async fn apply_write_action<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
    blufi_session: &mut BluFiSession,
    action: WriteAction,
) {
    match action {
        WriteAction::ProvisioningPsk => commit_provisioning(server, conn).await,
        WriteAction::Volume(level) => {
            log_audio_outcome("volume", crate::audio::persist_volume(level).await);
        }
        WriteAction::Mute(muted) => {
            log_audio_outcome("mute", crate::audio::persist_mute(muted).await);
        }
        WriteAction::Remote(cmd) => {
            defmt::info!("ble: remote command {}", defmt::Debug2Format(&cmd));
            REMOTE_COMMAND_SIGNAL.signal(cmd);
        }
        WriteAction::RemoteReset => {
            defmt::info!("ble: remote reset");
            REMOTE_COMMAND_SIGNAL.signal(stackchan_core::RemoteCommand::Reset);
        }
        WriteAction::CameraMode(active) => {
            defmt::info!("ble: camera_mode → {=bool}", active);
            snapshot::update_camera_mode(active);
            CAMERA_MODE_SIGNAL.signal(active);
        }
        WriteAction::CameraCapture => {
            defmt::info!("ble: camera_capture → signal queued");
            crate::camera::CAMERA_CAPTURE_REQUEST.signal(());
        }
        WriteAction::BluFiFrame(frame) => {
            log_blufi_frame(&frame);
            handle_blufi_frame(conn, blufi_session, frame).await;
        }
    }
}

/// Surface a parsed `BluFi` frame in defmt — type, subtype, sequence,
/// frame-control flags, and data length. No state mutation: the
/// SSID/password accumulator and the `ControlSubtype::ConnectToAp`
/// commit path land in a follow-up.
fn log_blufi_frame(frame: &blufi::Frame) {
    let kind = match frame.frame_type {
        blufi::Type::Control => "control",
        blufi::Type::Data => "data",
    };
    defmt::info!(
        "ble: blufi {=str} subtype={=u8:02x} seq={=u8} fc={=u8:02x} \
         encrypted={=bool} fragmented={=bool} data={=usize}B",
        kind,
        frame.subtype,
        frame.sequence,
        frame.frame_control,
        frame.encrypted,
        frame.fragmented,
        frame.data.len(),
    );
}

/// Dispatch a parsed `BluFi` frame to the session's state machine.
///
/// `Data` frames stage SSID / password into the session;
/// `ControlSubtype::ConnectToAp` runs the commit (auth-gated by the
/// shared [`apply_wifi_credentials`] helper); other subtypes are
/// already logged by [`log_blufi_frame`] and intentionally fall
/// through here — the device responds to them in slice 3.
///
/// Fragmented frames are rejected with a warn. The `BluFi` protocol
/// allows large payloads to be split across multiple application-
/// layer frames via the `FC_FRAGMENT` bit, and the GATT handler
/// owns reassembly per `stackchan_net::blufi`'s module contract.
/// Until reassembly lands (slice 3), staging only the trailing
/// fragment of a multi-frame `SendStaSsid` / `SendStaPassword`
/// would silently commit a truncated network name or passphrase —
/// the auth gate would happily accept it because the bytes are
/// still valid UTF-8. Rejecting cleanly here keeps the session
/// state coherent.
async fn handle_blufi_frame<P: PacketPool>(
    conn: &GattConnection<'_, '_, P>,
    session: &mut BluFiSession,
    frame: blufi::Frame,
) {
    if frame.fragmented {
        defmt::warn!(
            "ble: blufi fragmented frame (subtype={=u8:02x} seq={=u8}); \
             reassembly not implemented — dropping",
            frame.subtype,
            frame.sequence,
        );
        return;
    }
    match frame.frame_type {
        blufi::Type::Control => match frame.control_subtype() {
            Some(blufi::ControlSubtype::SetWifiOpMode) => {
                if let Some(&byte) = frame.data.first() {
                    session.op_mode = Some(byte);
                    defmt::info!("ble: blufi op_mode set ({=u8})", byte);
                } else {
                    defmt::warn!("ble: blufi op_mode payload empty; ignoring");
                }
            }
            Some(blufi::ControlSubtype::ConnectToAp) => {
                commit_blufi_provisioning(conn, session).await;
            }
            _ => {}
        },
        blufi::Type::Data => match frame.data_subtype() {
            Some(blufi::DataSubtype::SendStaSsid) => stage_blufi_ssid(session, &frame.data),
            Some(blufi::DataSubtype::SendStaPassword) => stage_blufi_psk(session, &frame.data),
            _ => {}
        },
    }
}

/// Stage an SSID from a `SendStaSsid` Data frame's payload.
///
/// SSIDs are 802.11 octet strings; the spec doesn't require UTF-8,
/// but the persistence layer ([`stackchan_net::config`]) is UTF-8-
/// only, so non-UTF-8 SSIDs are rejected here rather than coerced.
/// Over-cap SSIDs are rejected rather than truncated — silently
/// shortening a network name would commit the wrong AP.
fn stage_blufi_ssid(session: &mut BluFiSession, data: &[u8]) {
    let Ok(s) = core::str::from_utf8(data) else {
        defmt::warn!(
            "ble: blufi ssid not utf-8 ({=usize}B); ignoring",
            data.len()
        );
        return;
    };
    let mut staged: HString<PROV_SSID_CAP> = HString::new();
    if staged.push_str(s).is_err() {
        defmt::warn!(
            "ble: blufi ssid too long ({=usize}B > {=usize}); ignoring",
            s.len(),
            PROV_SSID_CAP
        );
        return;
    }
    session.staged_ssid = staged;
    defmt::info!(
        "ble: blufi staged ssid ({=usize} chars)",
        session.staged_ssid.len()
    );
}

/// Stage a PSK from a `SendStaPassword` Data frame's payload.
///
/// WPA2 passphrases are 8-63 ASCII chars per spec; that's a
/// strict subset of UTF-8 so the same validity rules apply.
/// Length-window enforcement happens at commit time
/// ([`apply_wifi_credentials`]), so an out-of-window PSK stages
/// fine here but fails commit cleanly.
///
/// Never logs PSK contents — only length.
fn stage_blufi_psk(session: &mut BluFiSession, data: &[u8]) {
    let Ok(s) = core::str::from_utf8(data) else {
        defmt::warn!("ble: blufi psk not utf-8 ({=usize}B); ignoring", data.len());
        return;
    };
    let mut staged: HString<PROV_PSK_CAP> = HString::new();
    if staged.push_str(s).is_err() {
        defmt::warn!(
            "ble: blufi psk too long ({=usize}B > {=usize}); ignoring",
            s.len(),
            PROV_PSK_CAP
        );
        return;
    }
    session.staged_psk = staged;
    defmt::info!(
        "ble: blufi staged psk ({=usize} chars)",
        session.staged_psk.len()
    );
}

/// Commit a `BluFi` `ConnectToAp` control frame: take the staged
/// SSID + password from the session and run them through the same
/// [`apply_wifi_credentials`] path the custom provisioning service
/// uses. The auth gate (the link must be `EncryptedAuthenticated`)
/// applies identically.
///
/// Clears `staged_psk` on success so a long-lived connection
/// doesn't keep the secret in RAM longer than necessary.
async fn commit_blufi_provisioning<P: PacketPool>(
    conn: &GattConnection<'_, '_, P>,
    session: &mut BluFiSession,
) {
    let committed = apply_wifi_credentials(
        conn,
        "blufi",
        session.staged_ssid.as_str(),
        session.staged_psk.as_str(),
    )
    .await;
    if committed {
        session.staged_psk = HString::new();
    }
}

/// Whether the connection is currently encrypted *and* authenticated
/// (passkey-confirmed bond). Plain `Encrypted` is the `JustWorks`
/// outcome — any nearby central can force that without the user
/// confirming a code, so it doesn't gate control writes.
fn is_authenticated<P: PacketPool>(conn: &GattConnection<'_, '_, P>) -> bool {
    matches!(
        conn.raw().security_level(),
        Ok(SecurityLevel::EncryptedAuthenticated)
    )
}

/// Single-line log for an audio persist outcome. Keeps the dispatch
/// arm a one-liner and centralises the warn-vs-info branching.
fn log_audio_outcome(field: &str, outcome: AudioPersistOutcome) {
    match outcome {
        AudioPersistOutcome::Persisted => {
            defmt::info!("ble: {} persisted", field);
        }
        AudioPersistOutcome::NoSnapshot => {
            defmt::warn!(
                "ble: {} write — config snapshot unavailable, dropping",
                field
            );
        }
        AudioPersistOutcome::NoStorage => {
            defmt::warn!("ble: {} write — no SD mounted, dropping", field);
        }
        AudioPersistOutcome::WriteFailed => {
            defmt::warn!("ble: {} write — SD persist failed, dropping", field);
        }
    }
}

/// Periodic battery notify (1 Hz) + snapshot-diff change notifies for
/// emotion / volume / mute.
///
/// Battery percent always notifies so first-time subscribers see a
/// value within a second. The other characteristics notify only on
/// transition — at steady state they barely change, and pushing a
/// tick whenever nothing moved would waste airtime and flicker in
/// nRF Connect. Reading the [`AvatarSnapshot`] each tick is the
/// single source of truth: it's already updated from the audio task
/// on persist + the render task on emotion change, so BLE just diffs.
async fn notify_task<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
    let battery_handle = server.battery.level;
    let emotion_handle = server.stackchan.emotion;
    let volume_handle = server.audio.volume;
    let mute_handle = server.audio.mute;
    let camera_mode_handle = server.view.camera_mode;
    let mut last_emotion: Option<Emotion> = None;
    let mut last_volume: Option<u8> = None;
    let mut last_mute: Option<bool> = None;
    let mut last_camera_mode: Option<bool> = None;

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
        notify_if_changed(
            server,
            conn,
            &emotion_handle,
            "emotion",
            &mut last_emotion,
            snap.emotion,
            &emotion_byte,
        )
        .await;

        let volume_byte = snap.audio.volume_pct;
        notify_if_changed(
            server,
            conn,
            &volume_handle,
            "volume",
            &mut last_volume,
            volume_byte,
            &volume_byte,
        )
        .await;

        let mute_byte = ble_command::encode_mute(snap.audio.muted);
        notify_if_changed(
            server,
            conn,
            &mute_handle,
            "mute",
            &mut last_mute,
            snap.audio.muted,
            &mute_byte,
        )
        .await;

        let camera_mode_byte = ble_command::encode_camera_mode(snap.camera_mode);
        notify_if_changed(
            server,
            conn,
            &camera_mode_handle,
            "camera_mode",
            &mut last_camera_mode,
            snap.camera_mode,
            &camera_mode_byte,
        )
        .await;

        Timer::after(BATTERY_NOTIFY_PERIOD).await;
    }
}

/// Update the GATT table value and emit a notify only when the
/// snapshot field changed since the last tick. Caller passes the
/// observed-state cache (`last`), the new state value (`current`),
/// and the byte payload to set + notify (already encoded).
async fn notify_if_changed<T, V, P>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
    handle: &Characteristic<V>,
    field: &str,
    last: &mut Option<T>,
    current: T,
    bytes: &V,
) where
    T: Copy + PartialEq,
    V: trouble_host::prelude::FromGatt,
    P: PacketPool,
{
    if let Err(e) = server.set(handle, bytes) {
        defmt::warn!(
            "ble: {} table set failed ({})",
            field,
            defmt::Debug2Format(&e)
        );
    }
    if *last == Some(current) {
        return;
    }
    *last = Some(current);
    if let Err(e) = handle.notify(conn, bytes).await {
        defmt::trace!(
            "ble: {} notify skipped ({}) — peer not subscribed?",
            field,
            defmt::Debug2Format(&e)
        );
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

/// Pre-fill read-side characteristic values from the avatar snapshot
/// so a phone reading them at connect time sees current state rather
/// than zeros. Covers the audio service (volume + mute) and the view
/// service (`camera_mode`). The notify loop later catches any change
/// made between this snapshot and the central's read.
fn populate_read_state(server: &StackchanServer<'_>) {
    let snap = snapshot::read();
    if let Err(e) = server.set(&server.audio.volume, &snap.audio.volume_pct) {
        defmt::warn!(
            "ble: audio volume initial set failed ({})",
            defmt::Debug2Format(&e)
        );
    }
    let mute_byte = ble_command::encode_mute(snap.audio.muted);
    if let Err(e) = server.set(&server.audio.mute, &mute_byte) {
        defmt::warn!(
            "ble: audio mute initial set failed ({})",
            defmt::Debug2Format(&e)
        );
    }
    let camera_mode_byte = ble_command::encode_camera_mode(snap.camera_mode);
    if let Err(e) = server.set(&server.view.camera_mode, &camera_mode_byte) {
        defmt::warn!(
            "ble: view camera_mode initial set failed ({})",
            defmt::Debug2Format(&e)
        );
    }
}

/// Commit a provisioning write: read staged SSID + just-written PSK
/// from the GATT table, hand them to [`apply_wifi_credentials`] for
/// the auth-gated persist + signal, then clear the PSK from the
/// table so a memory dump of the BLE stack doesn't leak the secret.
///
/// Failures inside `apply_wifi_credentials` are logged but never
/// panic — a malformed write should just be a no-op so the next
/// correct write can succeed.
async fn commit_provisioning<P: PacketPool>(
    server: &StackchanServer<'_>,
    conn: &GattConnection<'_, '_, P>,
) {
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

    // Return value intentionally dropped: the GATT-table PSK clear
    // below runs unconditionally, regardless of whether the commit
    // succeeded. The bool is only meaningful for the BluFi callsite,
    // which uses it to decide whether to scrub the in-memory session
    // copy. `apply_wifi_credentials` itself logs each outcome.
    let _ = apply_wifi_credentials(conn, "prov", staged_ssid.as_str(), new_psk.as_str()).await;

    // Clear the PSK from the GATT table unconditionally. Even on a
    // rejected commit the central just wrote the secret bytes into
    // the table; leaving them there for a subsequent retry or a
    // future maintenance change to the macro attributes (e.g.
    // accidentally enabling `read`) would let them leak.
    let empty: HString<PROV_PSK_CAP> = HString::new();
    if let Err(e) = server.set(&server.provisioning.psk, &empty) {
        defmt::warn!(
            "ble: prov: psk-clear table set ({})",
            defmt::Debug2Format(&e)
        );
    }
}

/// Auth-gated Wi-Fi-credential persist + signal. Shared between the
/// custom provisioning service ([`commit_provisioning`]) and the
/// `BluFi` `ConnectToAp` path ([`commit_blufi_provisioning`]). Each
/// call site stages credentials in its own way; this function owns
/// the auth check, validation, atomic SD writeback, snapshot update,
/// and `WIFI_RECONFIG` signal.
///
/// Gated on the connection's security level: unencrypted writes are
/// rejected with a warn. The trouble-host attribute server doesn't
/// enforce per-characteristic security in 0.5.1, so this app-layer
/// check is the load-bearing gate — moving it would let any nearby
/// BLE central reconfigure Wi-Fi without pairing.
///
/// `tag` is the defmt prefix the caller passes (`"prov"` or
/// `"blufi"`), so a log scrape can attribute the commit to its
/// source. Returns `true` iff the persist succeeded and
/// `WIFI_RECONFIG` was signalled.
async fn apply_wifi_credentials<P: PacketPool>(
    conn: &GattConnection<'_, '_, P>,
    tag: &'static str,
    staged_ssid: &str,
    new_psk: &str,
) -> bool {
    let level = match conn.raw().security_level() {
        Ok(l) => l,
        Err(e) => {
            defmt::warn!(
                "ble: {=str}: security_level query failed ({}); rejecting write",
                tag,
                defmt::Debug2Format(&e)
            );
            return false;
        }
    };
    // Require *authenticated* encryption (passkey-confirmed bond),
    // not bare `level.encrypted()`. Plain `Encrypted` is the
    // outcome of JustWorks pairing, which a central can force by
    // advertising `NoInputNoOutput` capabilities — the user never
    // sees a passkey, and there's no MITM protection. Allowing it
    // through here would let a phone next to the desk reconfigure
    // Wi-Fi without ever asking the user to confirm a code.
    if level != SecurityLevel::EncryptedAuthenticated {
        defmt::warn!(
            "ble: {=str}: write rejected — link not authenticated (level={}). \
             Pair the central with passkey confirmation before provisioning.",
            tag,
            defmt::Debug2Format(&level)
        );
        return false;
    }

    if staged_ssid.trim().is_empty() {
        defmt::warn!(
            "ble: {=str}: empty SSID — stage SSID before committing",
            tag
        );
        return false;
    }
    // Reject below-spec PSKs at the BLE boundary so they never reach
    // the SD card or the wifi task. Committing a 1–7 char PSK would
    // just thrash the radio on a retry loop. Empty PSK is allowed
    // for provisioning an open AP (which the 802.11 driver will
    // accept).
    if !new_psk.is_empty() && new_psk.len() < PROV_PSK_MIN {
        defmt::warn!(
            "ble: {=str}: PSK too short ({=usize} bytes; need {=usize}+)",
            tag,
            new_psk.len(),
            PROV_PSK_MIN
        );
        return false;
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
        defmt::warn!(
            "ble: {=str}: no config snapshot — boot config not yet read",
            tag
        );
        return false;
    };
    new_config.wifi.ssid = AString::from(staged_ssid);
    new_config.wifi.psk = AString::from(new_psk);

    let write_result = with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!(
                "ble: {=str}: persisted (ssid={=str})",
                tag,
                new_config.wifi.ssid.as_str()
            );
            *CONFIG_SNAPSHOT.lock().await = Some(new_config.clone());
            WIFI_RECONFIG.signal(WifiCreds {
                ssid: new_config.wifi.ssid,
                psk: new_config.wifi.psk,
            });
            true
        }
        Some(Err(e)) => {
            defmt::warn!("ble: {=str}: write_config failed ({})", tag, e);
            false
        }
        None => {
            defmt::warn!("ble: {=str}: no SD mounted — cannot persist", tag);
            false
        }
    }
}
