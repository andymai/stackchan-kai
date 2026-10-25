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
//! 6. `ota_flush` flips the `otadata` pointer and marks the new slot
//!    `NEW`, which the next boot treats as a trial.
//! 7. Soft-reset; the bootloader picks up the new slot on next boot.
//!
//! ## Trial boot and rollback
//!
//! espflash's bundled ESP-IDF bootloader is built without app-rollback
//! support, so the firmware runs the trial itself. [`check_boot_image`]
//! runs first thing in `main`: a slot still in trial whose previous
//! boot was a trial of the same slot and panicked (the crash latch)
//! is marked `INVALID` and the chip resets, and every ESP-IDF
//! bootloader skips an invalid `otadata` entry and boots the other
//! slot. A trial boot that survives
//! [`TRIAL_WINDOW_SECS`] is marked valid by [`confirm_after_trial`].
//! A hang without a panic isn't caught: `esp_hal::init` disables the
//! hardware watchdogs and the firmware doesn't re-arm them, so nothing
//! resets the chip.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::peripherals::FLASH;
use esp_hal_ota::{Ota, OtaImgState};
use esp_storage::FlashStorage;
use stackchan_net::ota::{OTA_PUBLIC_KEY_LEN, OtaImageError, parse_and_verify};

/// Build-time-baked Ed25519 public key (32 bytes), parsed from the
/// `STACKCHAN_OTA_PUBLIC_KEY` env var as 64 lowercase hex chars.
/// `None` disables OTA at compile time. Module-private — callers
/// use [`ota_enabled`] for the feature-flag check.
const OTA_PUBLIC_KEY: Option<[u8; OTA_PUBLIC_KEY_LEN]> =
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
/// from `main` via [`install_flash_peripheral`]. Every user goes
/// through [`with_parked_flash`], which hands a reborrow to a
/// short-lived `FlashStorage` and parks the peripheral again, so a
/// failed update can't strand it and block trial confirmation.
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

/// How long an OTA'd image has to run without panicking before it's
/// marked valid. A panic inside the window rolls it back on the next
/// boot.
pub const TRIAL_WINDOW_SECS: u64 = 60;

/// What [`check_boot_image`] found in the running slot.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, defmt::Format)]
pub enum BootImage {
    /// Validated by an earlier trial, or written over USB.
    Confirmed,
    /// An OTA'd image still inside its [`TRIAL_WINDOW_SECS`].
    Trial,
    /// The OTA state couldn't be read (no OTA partitions, flash error).
    Unknown,
}

impl BootImage {
    /// Wire name for `GET /health`'s `ota_image` field.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Trial => "trial",
            Self::Unknown => "unknown",
        }
    }
}

/// Current [`BootImage`], as its `repr(u8)` discriminant.
static BOOT_IMAGE: AtomicU8 = AtomicU8::new(BootImage::Unknown as u8);

/// Index of the running OTA app slot (`0` = `ota_0`), or [`NO_SLOT`].
static RUNNING_SLOT: AtomicU8 = AtomicU8::new(NO_SLOT);

/// [`RUNNING_SLOT`] sentinel for "not known".
const NO_SLOT: u8 = u8::MAX;

/// State of the running image, for `GET /health`.
#[must_use]
pub fn boot_image() -> BootImage {
    match BOOT_IMAGE.load(Ordering::Relaxed) {
        0 => BootImage::Confirmed,
        1 => BootImage::Trial,
        _ => BootImage::Unknown,
    }
}

/// OTA app slot the firmware is running from (`0` = `ota_0`), if known.
#[must_use]
pub fn running_slot() -> Option<u8> {
    match RUNNING_SLOT.load(Ordering::Relaxed) {
        NO_SLOT => None,
        slot => Some(slot),
    }
}

/// Set to [`TRIAL_MARK_BASE`] plus the slot index while a trial boot of
/// that slot runs. RTC fast RAM survives the `software_reset` a panic
/// triggers but not a power cycle, so on the next boot it tells a
/// crash of this trial apart from one in whatever image ran before it
/// (the old image can still panic between `ota_flush` and its reset).
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static TRIAL_MARK: portable_atomic::AtomicU32 = portable_atomic::AtomicU32::new(0);

/// High bits of a [`TRIAL_MARK`] value (`"TRL\0"`), so stale RTC RAM
/// after a cold boot is vanishingly unlikely to read as a mark.
const TRIAL_MARK_BASE: u32 = 0x5452_4C00;

/// Read the running slot's OTA state and roll back a crashed update.
///
/// Call before anything that could panic a bad image, with
/// `crashed_last_boot` set when the crash latch holds a panic from the
/// previous boot. Rolls back only when the slot is still in trial
/// (`NEW` from our own flush, or `PENDING_VERIFY` from a bootloader
/// with rollback support), the previous boot was a trial of this same
/// slot ([`TRIAL_MARK`]), and that boot panicked. The slot is then
/// marked `INVALID` and the chip resets into the other slot, leaving
/// the crash latch for the recovered image to save. Never returns in
/// that case.
pub fn check_boot_image(flash: FLASH<'_>, crashed_last_boot: bool) -> BootImage {
    let mut ota = match Ota::new(FlashStorage::new(flash)) {
        Ok(ota) => ota,
        Err(e) => {
            defmt::warn!(
                "ota: no OTA partition layout ({}); boot-image check skipped",
                defmt::Debug2Format(&e)
            );
            return BootImage::Unknown;
        }
    };
    let slot = ota
        .get_currently_booted_partition()
        .and_then(|p| u8::try_from(p).ok());
    if let Some(slot) = slot {
        RUNNING_SLOT.store(slot, Ordering::Relaxed);
    }
    let mark = slot.map(|s| TRIAL_MARK_BASE | u32::from(s));
    let image = match ota.get_ota_image_state() {
        Ok(OtaImgState::EspOtaImgNew | OtaImgState::EspOtaImgPendingVerify) => {
            let trial_of_this_slot = mark.is_some_and(|m| TRIAL_MARK.load(Ordering::Relaxed) == m);
            if crashed_last_boot && trial_of_this_slot {
                defmt::error!("ota: update panicked during its trial; rolling back");
                TRIAL_MARK.store(0, Ordering::Relaxed);
                match ota.ota_mark_app_invalid_rollback() {
                    Ok(()) => esp_hal::system::software_reset(),
                    Err(e) => defmt::error!(
                        "ota: rollback failed ({}); staying on this image",
                        defmt::Debug2Format(&e)
                    ),
                }
            }
            TRIAL_MARK.store(mark.unwrap_or(0), Ordering::Relaxed);
            BootImage::Trial
        }
        Ok(_) => {
            TRIAL_MARK.store(0, Ordering::Relaxed);
            BootImage::Confirmed
        }
        Err(e) => {
            defmt::warn!(
                "ota: running slot not found in otadata ({})",
                defmt::Debug2Format(&e)
            );
            BootImage::Unknown
        }
    };
    BOOT_IMAGE.store(image as u8, Ordering::Relaxed);
    defmt::info!(
        "ota: running slot {} ({})",
        running_slot(),
        image.wire_str()
    );
    image
}

/// Mark the running slot valid once it has run for
/// [`TRIAL_WINDOW_SECS`]. Spawn only for a [`BootImage::Trial`] boot.
pub async fn confirm_after_trial() {
    embassy_time::Timer::after_secs(TRIAL_WINDOW_SECS).await;
    let marked = with_parked_flash(|flash| {
        Ota::new(FlashStorage::new(flash)).and_then(|mut ota| ota.ota_mark_app_valid())
    });
    match marked {
        Some(Ok(())) => {
            TRIAL_MARK.store(0, Ordering::Relaxed);
            BOOT_IMAGE.store(BootImage::Confirmed as u8, Ordering::Relaxed);
            defmt::info!("ota: trial passed; running slot marked valid");
        }
        Some(Err(e)) => defmt::warn!(
            "ota: marking the running slot valid failed ({})",
            defmt::Debug2Format(&e)
        ),
        None => defmt::warn!("ota: flash peripheral never parked; slot left in trial"),
    }
}

/// Run `f` on a reborrow of the parked FLASH peripheral, then park it
/// again. `None` only when [`install_flash_peripheral`] never ran: the
/// executor is cooperative and `f` is synchronous, so no other user
/// can hold the peripheral at the same time.
fn with_parked_flash<R>(f: impl FnOnce(FLASH<'_>) -> R) -> Option<R> {
    let mut flash = FLASH_SLOT.lock(|cell| cell.borrow_mut().take())?;
    let result = f(flash.reborrow());
    FLASH_SLOT.lock(|cell| {
        cell.borrow_mut().replace(flash);
    });
    Some(result)
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

    let crc = esp_rom_sys::rom::crc::crc32_le(0, payload);
    with_parked_flash(|flash| write_inactive_slot(flash, payload, crc))
        .unwrap_or(Err(OtaPerformError::FlashUnavailable))
}

/// Stream `payload` into the inactive OTA slot and arm it for a trial
/// boot. Split out of [`perform_update`] so every `?` returns through
/// [`with_parked_flash`], which re-parks the peripheral on failure.
fn write_inactive_slot(flash: FLASH<'_>, payload: &[u8], crc: u32) -> Result<(), OtaPerformError> {
    let mut ota = Ota::new(FlashStorage::new(flash))?;
    let payload_len: u32 = u32::try_from(payload.len())
        .map_err(|_| OtaPerformError::Flash(esp_hal_ota::OtaError::OtaPartitionTooSmall))?;
    ota.ota_begin(payload_len, crc)?;
    for chunk in payload.chunks(FLASH_CHUNK_BYTES) {
        ota.ota_write_chunk(chunk)?;
    }
    ota.ota_flush(true, true)?;
    defmt::info!("ota: flush done; new slot armed for a trial boot");
    Ok(())
}
