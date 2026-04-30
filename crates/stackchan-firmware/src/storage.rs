//! SD-card boot config storage.
//!
//! Reads `/sd/STACKCHAN.RON` into a [`stackchan_net::Config`] at
//! boot; writes back atomically on `PUT /settings`. Falls back to
//! [`Config::default`] on any failure so the avatar still boots
//! offline-first when the SD is missing or the file is malformed.
//!
//! ## Bus sharing
//!
//! [`Storage`] borrows the shared SPI2 bus through the [`SdSpiDevice`]
//! adapter, which flips GPIO35's output-enable bit so the LCD's DC
//! line and the SD's MISO line can coexist on the same physical pin.
//! See `sd_spi.rs` for the M5GFX-derived OE-flip pattern.
//!
//! [`Config`]: stackchan_net::Config
//! [`Config::default`]: stackchan_net::Config::default

use alloc::vec::Vec;
use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_hal::digital::OutputPin;
use embedded_sdmmc::{Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use esp_hal::Blocking;
use esp_hal::gpio::Output;
use esp_hal::spi::master::Spi;

use crate::sd_spi::SdSpiDevice;

/// Concrete `Storage` instantiation used by the firmware. CS is the
/// SD chip-select pin (GPIO4 on CoreS3).
pub type FirmwareStorage = Storage<Output<'static>>;

/// `Send` wrapper for [`FirmwareStorage`].
///
/// `FirmwareStorage` is `!Send` because its underlying
/// [`crate::sd_spi::SdSpiDevice`] holds a `&'static RefCell<Spi>`
/// (the SPI2 bus shared with the LCD), and `&RefCell<_>` is `!Send`
/// for cross-thread aliasing reasons.
///
/// In this firmware those reasons don't apply: the embassy executor
/// runs on a single core in a single thread, and bus access is
/// already serialized at the `RefCellDevice` layer. Holding the
/// storage in a static [`Mutex`] further enforces single-task access.
///
/// We document the invariant in this wrapper so the unsafe stays
/// scoped to one place rather than leaking into every static-borrow
/// call site.
struct StorageHolder(Option<FirmwareStorage>);

// SAFETY: single-core embassy executor — no cross-thread aliasing.
// Bus access is serialized via `RefCellDevice` (LCD side) and via
// the enclosing `Mutex` (storage side). See `StorageHolder` doc.
#[allow(
    unsafe_code,
    clippy::non_send_fields_in_send_ty,
    reason = "see StorageHolder doc for the single-core invariant"
)]
unsafe impl Send for StorageHolder {}

/// Mounted SD-card storage, populated at boot when the card is
/// present and FAT-mountable. `None` when the card is missing or
/// the mount failed — `PUT /settings` returns `503` in that case.
///
/// Async mutex (yields under contention) because SD writes can take
/// tens of ms and we don't want to hold the lock across a render
/// frame. Access through [`install_storage`] / [`with_storage`].
static SHARED_STORAGE: Mutex<CriticalSectionRawMutex, StorageHolder> =
    Mutex::new(StorageHolder(None));

/// Latest persisted [`stackchan_net::Config`].
///
/// Initialised at boot from the SD read (or
/// [`stackchan_net::Config::default`] when the card is missing);
/// updated on each successful `PUT /settings`. `GET /settings`
/// reads from this to avoid re-reading the SD on every request.
///
/// `None` only during the brief window between firmware start and
/// the boot path's first write — HTTP isn't accepting requests yet,
/// so consumers see `Some(_)` for the lifetime of the run.
pub static CONFIG_SNAPSHOT: Mutex<CriticalSectionRawMutex, Option<stackchan_net::Config>> =
    Mutex::new(None);

/// Move `storage` into [`SHARED_STORAGE`], replacing whatever was
/// there. Called once from the boot path after a successful mount.
pub async fn install_storage(storage: FirmwareStorage) {
    SHARED_STORAGE.lock().await.0 = Some(storage);
}

/// Run `f` against the currently installed [`FirmwareStorage`], if
/// any. Returns `None` when no SD is mounted — callers should map
/// that to a `503 Service Unavailable` response.
pub async fn with_storage<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut FirmwareStorage) -> R,
{
    let mut guard = SHARED_STORAGE.lock().await;
    guard.0.as_mut().map(f)
}

/// Filename written to / read from the FAT root.
const CONFIG_FILE: &str = "STACKCHAN.RON";

/// Atomic-write staging name. Written first, then rename-copied onto
/// `STACKCHAN.RON`; mid-write power loss leaves the old file intact.
const STAGING_FILE: &str = "STACKCHAN.NEW";

/// Cap on the RON we'll read into memory at boot. Schema v1 fits
/// well under 1 KiB; the headroom keeps SRAM bounded if the schema
/// grows.
const MAX_CONFIG_BYTES: u32 = 4096;

/// Filename for persisted BLE bonds. Binary format defined in
/// `crate::ble::bonds`.
const BONDS_FILE: &str = "BONDS.BIN";

/// Staging filename for atomic bond writes — same dance as the
/// config file: write to `BONDS.NEW`, copy onto `BONDS.BIN`, delete
/// the staging file.
const BONDS_STAGING_FILE: &str = "BONDS.NEW";

/// Cap on the bonds blob. trouble-host's `BI_COUNT` is 10 bonds;
/// our record format is ~50 bytes per bond. 1 KiB leaves generous
/// headroom for future record-layout additions.
const MAX_BONDS_BYTES: u32 = 1024;

/// Filename for the operator-triggered camera capture. Single fixed
/// name (no rotation) so each capture overwrites the previous —
/// the workflow is "trigger, eject SD, view, repeat", not a
/// timestamped archive. Raw QVGA RGB565, big-endian, 320×240×2 =
/// 153 600 bytes; convertible with a one-line numpy reshape.
const CAPTURE_FILE: &str = "CAPTURE.565";

/// Storage / FAT errors. Logged via `defmt::Format` at the firmware
/// boundary; the operator triages from the boot log.
#[derive(Debug, defmt::Format)]
#[non_exhaustive]
pub enum StorageError {
    /// SD-side SPI error (bus glitch, OE mis-flip, card not present).
    Spi,
    /// Card init / `num_bytes` failure.
    CardInit,
    /// Volume open / root-dir traversal failure.
    Volume,
    /// `STACKCHAN.RON` (or staging file) not present on the FAT root.
    FileNotFound,
    /// File read failed mid-stream.
    Read,
    /// File write or flush failed mid-stream.
    Write,
    /// Config bytes exceeded `MAX_CONFIG_BYTES`.
    TooLarge,
    /// File contents weren't valid UTF-8.
    NotUtf8,
    /// `stackchan_net::parse_ron_bare` rejected the file. Inner detail
    /// logged via `defmt::Debug2Format` at the call site, not carried
    /// here.
    Decode,
}

/// Stub `TimeSource` — embedded-sdmmc requires one to stamp FAT
/// directory entries. Until a wall-clock source is wired in, every
/// entry gets the FAT epoch (1980-01-01 00:00:00).
struct EpochTime;

impl TimeSource for EpochTime {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 10, // 1980, the FAT epoch
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

/// Concrete `VolumeManager` instantiation we use throughout. Type
/// alias keeps the firmware's `main.rs` storage-handle declaration
/// readable.
type SdMgr<CS> = VolumeManager<SdCard<SdSpiDevice<'static, CS, Delay>, Delay>, EpochTime>;

/// Mounted SD-card storage handle. Reads / writes the boot config RON.
///
/// Construct via [`Storage::mount`]. Methods take `&mut self` because
/// `embedded-sdmmc 0.8`'s `VolumeManager` needs mutable access for
/// every operation (cluster-chain walks, FAT cache, open-handle book-
/// keeping).
pub struct Storage<CS>
where
    CS: OutputPin,
{
    /// `embedded-sdmmc` owns the underlying SD card driver + the
    /// time-source stub. We hold one `VolumeManager` for the lifetime
    /// of the firmware run.
    mgr: SdMgr<CS>,
}

impl<CS> Storage<CS>
where
    CS: OutputPin,
{
    /// Initialize the SD card and confirm a FAT volume exists.
    ///
    /// `bus` is the shared SPI2 `RefCell` set up at LCD bring-up;
    /// `cs` is the SD-side chip-select (GPIO4 on CoreS3).
    ///
    /// # Errors
    ///
    /// [`StorageError::CardInit`] if the SD doesn't respond to
    /// initialisation, [`StorageError::Volume`] if FAT mount fails.
    pub fn mount(
        bus: &'static RefCell<Spi<'static, Blocking>>,
        cs: CS,
    ) -> Result<Self, StorageError> {
        let sd_device = SdSpiDevice::new(bus, cs, Delay);
        let sd_card = SdCard::new(sd_device, Delay);
        // Force card init by querying capacity. Returns once the SD
        // has answered ACMD41 and entered the data state.
        sd_card.num_bytes().map_err(|_| StorageError::CardInit)?;
        let mut mgr = VolumeManager::new(sd_card, EpochTime);
        // Probe FAT volume 0 and immediately drop the handle — we
        // re-open per call so callers don't have to thread lifetimes.
        // Explicit `drop` so the borrow on `mgr` ends before we move
        // the manager into `Self`.
        let probe = mgr
            .open_volume(VolumeIdx(0))
            .map_err(|_| StorageError::Volume)?;
        drop(probe);
        Ok(Self { mgr })
    }

    /// Read `/sd/STACKCHAN.RON` and parse it into a [`stackchan_net::Config`].
    ///
    /// # Errors
    ///
    /// [`StorageError::FileNotFound`] if the file is missing,
    /// [`StorageError::Read`] on a partial / failed read,
    /// [`StorageError::TooLarge`] if the file exceeds `MAX_CONFIG_BYTES`,
    /// [`StorageError::NotUtf8`] / [`StorageError::Decode`] on parse failure.
    pub fn read_config(&mut self) -> Result<stackchan_net::Config, StorageError> {
        let mut volume = self
            .mgr
            .open_volume(VolumeIdx(0))
            .map_err(|_| StorageError::Volume)?;
        let mut root = volume.open_root_dir().map_err(|_| StorageError::Volume)?;
        let mut file = root
            .open_file_in_dir(CONFIG_FILE, Mode::ReadOnly)
            .map_err(|_| StorageError::FileNotFound)?;

        let len = file.length();
        if len > MAX_CONFIG_BYTES {
            return Err(StorageError::TooLarge);
        }
        let len = len as usize;
        let mut buf = alloc::vec![0u8; len];
        let n = file.read(&mut buf).map_err(|_| StorageError::Read)?;
        buf.truncate(n);

        let text = core::str::from_utf8(&buf).map_err(|_| StorageError::NotUtf8)?;
        stackchan_net::parse_ron_bare(text).map_err(|e| {
            defmt::warn!("config parse failed: {}", defmt::Debug2Format(&e));
            StorageError::Decode
        })
    }

    /// Atomically replace `/sd/STACKCHAN.RON` with the rendered RON
    /// of `config`. Writes to `STACKCHAN.NEW` first, then copies the
    /// bytes onto `STACKCHAN.RON` and deletes the staging file.
    ///
    /// # Errors
    ///
    /// [`StorageError::Write`] on any underlying write failure,
    /// [`StorageError::Decode`] if the round-trip render itself
    /// fails (should not happen with a well-formed `Config`).
    pub fn write_config(&mut self, config: &stackchan_net::Config) -> Result<(), StorageError> {
        let rendered = stackchan_net::render_ron_bare(config).map_err(|_| StorageError::Decode)?;
        self.write_file(STAGING_FILE, rendered.as_bytes())?;
        self.copy_then_delete(STAGING_FILE, CONFIG_FILE)?;
        Ok(())
    }

    /// Read the full BLE bonds blob from `/sd/BONDS.BIN`. Returns
    /// `Ok(empty)` if the file is missing — that's the first-boot
    /// state, not an error. Layout is opaque here; the BLE module
    /// owns the byte format.
    ///
    /// # Errors
    ///
    /// [`StorageError::Read`] on a partial read,
    /// [`StorageError::TooLarge`] if the file exceeds
    /// [`MAX_BONDS_BYTES`].
    pub fn read_bonds(&mut self) -> Result<Vec<u8>, StorageError> {
        let mut volume = self
            .mgr
            .open_volume(VolumeIdx(0))
            .map_err(|_| StorageError::Volume)?;
        let mut root = volume.open_root_dir().map_err(|_| StorageError::Volume)?;
        let Ok(mut file) = root.open_file_in_dir(BONDS_FILE, Mode::ReadOnly) else {
            return Ok(Vec::new());
        };
        let len = file.length();
        if len > MAX_BONDS_BYTES {
            return Err(StorageError::TooLarge);
        }
        let len = len as usize;
        let mut buf = alloc::vec![0u8; len];
        let n = file.read(&mut buf).map_err(|_| StorageError::Read)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Truncate-write the operator-triggered camera capture into
    /// `/sd/CAPTURE.565`. No staging dance — a half-written capture
    /// is just a half-written capture; the next trigger overwrites
    /// it. The atomicity that matters for `STACKCHAN.RON` /
    /// `BONDS.BIN` (boot must read a complete file) doesn't apply
    /// here.
    ///
    /// # Errors
    ///
    /// [`StorageError::Write`] on any underlying SPI / FAT failure.
    pub fn write_capture(&mut self, frame: &[u8]) -> Result<(), StorageError> {
        self.write_file(CAPTURE_FILE, frame)
    }

    /// Atomically replace `/sd/BONDS.BIN` with `data`. Same staging-
    /// then-copy dance the config writeback uses; mid-write power
    /// loss leaves the previous bonds file intact.
    ///
    /// # Errors
    ///
    /// [`StorageError::Write`] on any underlying write failure,
    /// [`StorageError::TooLarge`] if `data` exceeds
    /// [`MAX_BONDS_BYTES`].
    pub fn write_bonds(&mut self, data: &[u8]) -> Result<(), StorageError> {
        // `usize > MAX_BONDS_BYTES as usize` is the strictly-stronger
        // form: covers both targets where `usize` is 32 bits (firmware,
        // matches u32 semantics) and 64 bits (host doctests, where a
        // length above `u32::MAX` would have silently bypassed an
        // earlier `try_from`-based guard).
        if data.len() > MAX_BONDS_BYTES as usize {
            return Err(StorageError::TooLarge);
        }
        self.write_file(BONDS_STAGING_FILE, data)?;
        self.copy_then_delete(BONDS_STAGING_FILE, BONDS_FILE)?;
        Ok(())
    }

    /// Truncate-write `data` into `name`.
    fn write_file(&mut self, name: &str, data: &[u8]) -> Result<(), StorageError> {
        let mut volume = self
            .mgr
            .open_volume(VolumeIdx(0))
            .map_err(|_| StorageError::Volume)?;
        let mut root = volume.open_root_dir().map_err(|_| StorageError::Volume)?;
        let mut file = root
            .open_file_in_dir(name, Mode::ReadWriteCreateOrTruncate)
            .map_err(|_| StorageError::Write)?;
        file.write(data).map_err(|_| StorageError::Write)?;
        file.flush().map_err(|_| StorageError::Write)?;
        Ok(())
    }

    /// Copy `from`'s bytes into `to` (truncating any prior `to`),
    /// then delete `from`. embedded-sdmmc 0.8 has no first-class
    /// rename, so we do the copy-and-delete dance — the cost is
    /// the file's read+write twice, which for the schema-v1 config
    /// (<1 KiB) is negligible.
    fn copy_then_delete(&mut self, from: &str, to: &str) -> Result<(), StorageError> {
        let mut volume = self
            .mgr
            .open_volume(VolumeIdx(0))
            .map_err(|_| StorageError::Volume)?;
        let mut root = volume.open_root_dir().map_err(|_| StorageError::Volume)?;

        let staged: Vec<u8> = {
            let mut src = root
                .open_file_in_dir(from, Mode::ReadOnly)
                .map_err(|_| StorageError::Write)?;
            let len = src.length();
            if len > MAX_CONFIG_BYTES {
                return Err(StorageError::TooLarge);
            }
            let mut buf = alloc::vec![0u8; len as usize];
            let n = src.read(&mut buf).map_err(|_| StorageError::Write)?;
            buf.truncate(n);
            buf
        };

        {
            let mut dst = root
                .open_file_in_dir(to, Mode::ReadWriteCreateOrTruncate)
                .map_err(|_| StorageError::Write)?;
            dst.write(&staged).map_err(|_| StorageError::Write)?;
            dst.flush().map_err(|_| StorageError::Write)?;
        }

        // Best-effort delete; if the staging file is missing we still
        // succeeded at the atomic-copy goal.
        let _ = root.delete_file_in_dir(from);

        // Defeat the unused-variable hint without an explicit drop —
        // `staged` is a `Vec` we explicitly want to release at end of
        // scope, after the destination write completed.
        drop(staged);
        Ok(())
    }
}

/// Lightweight summary printed at boot. Avoids leaking the PSK in
/// the firmware's defmt log.
pub fn log_config_summary(config: &stackchan_net::Config) {
    defmt::info!(
        "net config: ssid={=str} country={=str} hostname={=str}.local",
        config.wifi.ssid.as_str(),
        config.wifi.country.as_str(),
        config.mdns.hostname.as_str(),
    );
}
