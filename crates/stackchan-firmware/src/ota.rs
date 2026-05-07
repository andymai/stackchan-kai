//! On-device OTA update path: parse + verify the SCFW envelope and
//! stream-write the payload to the inactive flash slot.
//!
//! Layered on top of:
//!
//! - [`stackchan_net::ota::parse_and_verify`] for the SCFW framing +
//!   ed25519 signature check (host-tested, deterministic).
//! - [`esp_hal_ota::Ota`] for the partition-table parse + slot
//!   selection + `otadata` pointer dance the bootloader needs to
//!   pick up the new image on next boot.
//!
//! ## Build-time public key
//!
//! The verifier's public key is baked into the firmware at compile
//! time via the `STACKCHAN_OTA_PUBLIC_KEY` environment variable —
//! 64 hex characters (32 bytes). When the variable is unset, OTA is
//! disabled at compile time: [`ota_enabled`] returns `false` and the
//! HTTP route returns `503 Service Unavailable`. This is the
//! offline-first stance: you can build + flash without an OTA
//! signing keypair set up; you just can't do field updates.
//!
//! ## Why the whole image is buffered before flashing
//!
//! Ed25519 signs the message in full — verification can't proceed
//! incrementally without switching to Ed25519ph (pre-hashed). Until
//! that's worth the schema bump, the verifier needs the payload
//! contiguous in memory. CoreS3 has 8 MB of PSRAM; even a 4 MB
//! image leaves comfortable headroom for the running heap.
//!
//! ## Flash-write order
//!
//! 1. Allocate a `Vec<u8>` on PSRAM sized for the request body.
//! 2. Read the body off the socket in chunks.
//! 3. `parse_and_verify` against the build-time public key. Reject
//!    with `400` / `403` if framing or signature fails.
//! 4. Compute CRC32 of the payload (esp-hal-ota requires it for
//!    `ota_begin`; mirrors what the IDF bootloader uses).
//! 5. Stream the verified payload through `ota_write_chunk` into the
//!    inactive slot.
//! 6. `ota_flush` flips the `otadata` pointer.
//! 7. Soft-reset; the bootloader picks up the new slot on next boot.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::peripherals::FLASH;
use esp_hal_ota::Ota;
use esp_storage::FlashStorage;
use stackchan_net::ota::{OTA_PUBLIC_KEY_LEN, OtaImageError, parse_and_verify};

/// Build-time-baked Ed25519 public key (32 bytes), parsed from the
/// `STACKCHAN_OTA_PUBLIC_KEY` env var as 64 lowercase hex chars.
/// `None` disables OTA at compile time.
pub const OTA_PUBLIC_KEY: Option<[u8; OTA_PUBLIC_KEY_LEN]> =
    match option_env!("STACKCHAN_OTA_PUBLIC_KEY") {
        Some(hex) => Some(decode_hex_pubkey(hex)),
        None => None,
    };

/// True iff OTA is wired up — i.e. the build-time public key is
/// present. Used by the HTTP route to fail closed when the firmware
/// was built without OTA support.
#[must_use]
pub const fn ota_enabled() -> bool {
    OTA_PUBLIC_KEY.is_some()
}

/// Decode a 64-character hex string into a 32-byte array at compile
/// time. The build fails through a const-eval assertion if the input
/// is malformed — better a clear compile-time message than silently
/// shipping a bad key.
#[allow(
    clippy::panic,
    reason = "build-time const eval — a panic here surfaces as a clear compile error"
)]
const fn decode_hex_pubkey(hex: &str) -> [u8; OTA_PUBLIC_KEY_LEN] {
    let b = hex.as_bytes();
    assert!(
        b.len() == 64,
        "STACKCHAN_OTA_PUBLIC_KEY must be exactly 64 hex chars"
    );
    let mut out = [0u8; OTA_PUBLIC_KEY_LEN];
    let mut i = 0;
    while i < OTA_PUBLIC_KEY_LEN {
        out[i] = (hex_nibble(b[i * 2]) << 4) | hex_nibble(b[i * 2 + 1]);
        i += 1;
    }
    out
}

/// Const-evaluable hex-nibble decoder. Const-panics on a non-hex
/// byte so the build fails at the malformed character.
#[allow(
    clippy::panic,
    reason = "build-time const eval — a panic here surfaces as a clear compile error"
)]
const fn hex_nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("STACKCHAN_OTA_PUBLIC_KEY contains a non-hex character"),
    }
}

/// Slot for the boot-acquired `FLASH` peripheral. Populated once
/// from `main` via [`install_flash_peripheral`]; consumed by
/// [`perform_update`] (a successful update reboots, so the flash
/// handle is never returned).
static FLASH_SLOT: Mutex<CriticalSectionRawMutex, RefCell<Option<FLASH<'static>>>> =
    Mutex::new(RefCell::new(None));

/// Park the FLASH peripheral so the OTA path can claim it later.
/// Called once from boot; calling twice is a programming error and
/// silently overwrites the previous handle.
pub fn install_flash_peripheral(flash: FLASH<'static>) {
    FLASH_SLOT.lock(|cell| {
        cell.borrow_mut().replace(flash);
    });
    defmt::info!("ota: flash peripheral parked for OTA writer");
}

/// Outcome of [`perform_update`].
#[derive(Debug, defmt::Format)]
pub enum OtaPerformError {
    /// Build-time public key was not set; OTA is compiled out.
    Disabled,
    /// FLASH peripheral was never parked at boot — OTA is unreachable.
    /// Programming error: `install_flash_peripheral` not called.
    FlashUnavailable,
    /// SCFW framing or ed25519 verification failed.
    Image(#[defmt(Debug2Format)] OtaImageError),
    /// `esp-hal-ota` rejected the partition setup or chunk write.
    Flash(#[defmt(Debug2Format)] esp_hal_ota::OtaError),
}

impl From<OtaImageError> for OtaPerformError {
    fn from(e: OtaImageError) -> Self {
        Self::Image(e)
    }
}

impl From<esp_hal_ota::OtaError> for OtaPerformError {
    fn from(e: esp_hal_ota::OtaError) -> Self {
        Self::Flash(e)
    }
}

/// Chunk size for the streaming write into the inactive OTA slot.
/// 4 KiB matches the ESP32 flash sector size, keeping the inner
/// `ota_write_chunk` call sector-aligned for whatever buffering
/// `esp-hal-ota` does internally.
const FLASH_CHUNK_BYTES: usize = 4096;

/// Parse + verify the SCFW image, then stream the payload into the
/// inactive OTA slot and flip the `otadata` pointer.
///
/// On success, the **caller** is responsible for the soft-reset:
/// the function returns so the HTTP handler can drain its `200 OK`
/// response before the chip resets, otherwise the dashboard sees a
/// TCP RST instead of the success body. See [`crate::net::http`]'s
/// `/restart` handler for the same pattern.
///
/// # Errors
///
/// - [`OtaPerformError::Disabled`] if the firmware was built without
///   `STACKCHAN_OTA_PUBLIC_KEY`.
/// - [`OtaPerformError::Image`] if framing fails or the signature
///   doesn't verify.
/// - [`OtaPerformError::Flash`] for partition / write failures.
pub fn perform_update(image_bytes: &[u8]) -> Result<(), OtaPerformError> {
    let Some(public_key) = OTA_PUBLIC_KEY.as_ref() else {
        return Err(OtaPerformError::Disabled);
    };
    let payload = parse_and_verify(image_bytes, public_key)?;
    defmt::info!(
        "ota: signature verified, payload {=usize} bytes — streaming to inactive slot",
        payload.len()
    );

    let crc = esp_hal_ota::crc32::calc_crc32(payload, 0);
    let flash_periph = FLASH_SLOT
        .lock(|cell| cell.borrow_mut().take())
        .ok_or(OtaPerformError::FlashUnavailable)?;
    let flash = FlashStorage::new(flash_periph);
    let mut ota = Ota::new(flash)?;
    let payload_len: u32 = u32::try_from(payload.len())
        .map_err(|_| OtaPerformError::Flash(esp_hal_ota::OtaError::OtaPartitionTooSmall))?;
    ota.ota_begin(payload_len, crc)?;
    for chunk in payload.chunks(FLASH_CHUNK_BYTES) {
        ota.ota_write_chunk(chunk)?;
    }
    ota.ota_flush(true, false)?;
    defmt::info!("ota: flush done; new slot armed for next boot");
    Ok(())
}
