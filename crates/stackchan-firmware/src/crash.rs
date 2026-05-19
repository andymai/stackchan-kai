//! Persistent crash latch in RTC fast RAM + boot-time SD save.
//!
//! The default `#[panic_handler]` for a `no_std` firmware is "log
//! over defmt and `loop {}`" — which strands the device with no
//! record once it's plugged into a wall and the defmt console isn't
//! attached. This module captures the panic message + location into
//! RTC fast RAM (preserved across software resets, watchdog
//! timeouts, and the bootloader handoff via
//! `#[esp_hal::ram(unstable(rtc_fast, persistent))]`), reboots the
//! chip, and on the *next* boot copies the latch contents into
//! `/sd/CRASH.LOG` so the operator can fetch them via `GET /crash`
//! or the dashboard.
//!
//! ## Lifecycle
//!
//! 1. Panic handler calls [`record_panic`], which serialises
//!    [`PanicInfo`] into [`CRASH_LATCH`] (the persistent RAM buffer)
//!    via [`stackchan_net::crash_latch::encode`].
//! 2. Panic handler calls `esp_hal::system::software_reset()`. The
//!    chip restarts; RTC fast RAM is preserved across the reset.
//! 3. On the next boot, [`consume_latch`] validates magic +
//!    checksum and returns the snapshot, atomically clearing the
//!    magic so the same crash isn't replayed every subsequent boot.
//! 4. The boot path in `main.rs` writes the snapshot to
//!    `/sd/CRASH.LOG` once SD is mounted; `GET /crash` returns it,
//!    `POST /crash/clear` deletes it.
//!
//! ## Why RTC fast RAM and not flash?
//!
//! Flash writes from a panic context are dangerous: the SPI flash
//! controller may be mid-operation, the heap may be poisoned, and a
//! partial write could brick the running firmware image. RTC fast
//! RAM is a small dedicated SRAM region the CPU writes atomically
//! without involving any peripheral driver. The `persistent` mode of
//! `#[ram]` skips the zero-init that runs on every boot, so the
//! latch survives `software_reset` even though the rest of the SRAM
//! is reinitialised.
//!
//! Byte layout + encode / decode helpers live in
//! [`stackchan_net::crash_latch`] so they're host-testable. This
//! module keeps the `static mut` and the `#[panic_handler]` glue.

// Reaches the `static mut CRASH_LATCH` directly. The two `unsafe`
// blocks below are the only places the latch buffer is touched at
// runtime: once from the panic handler (every other CPU activity is
// suspended; `RECORDING` serialises re-entrant panics) and once
// from `main` before any task has spawned (single-threaded by
// construction). No async path or interrupt handler touches the
// latch, so there's no observable aliasing.
#![allow(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use stackchan_net::crash_latch::{LATCH_SIZE, clear_magic, decode, encode, format_into};

pub use stackchan_net::crash_latch::CrashSnapshot;

/// Persistent crash-latch buffer. RTC fast RAM is preserved across
/// `software_reset` thanks to the `persistent` mode of
/// `#[esp_hal::ram]`, which skips the zero-init normally performed
/// at every boot.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
pub static mut CRASH_LATCH: [u8; LATCH_SIZE] = [0u8; LATCH_SIZE];

/// Re-entrancy guard — flips to `true` on the first call to
/// [`record_panic`]. A panic that happens *inside* the panic handler
/// (e.g. a recursive call through a corrupt formatter) becomes a
/// no-op, so the original snapshot survives instead of being
/// overwritten with garbage.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// Capture `info` into [`CRASH_LATCH`]. Idempotent against
/// re-entrant panics — the first call wins.
///
/// # Safety
///
/// Touches the `static mut CRASH_LATCH`. Panic handlers run with
/// every other CPU activity suspended, and `RECORDING` serialises
/// re-entrant calls.
pub fn record_panic(info: &PanicInfo<'_>) {
    if RECORDING.swap(true, Ordering::SeqCst) {
        return;
    }

    // Render the panic message into a stack buffer. `format!`
    // requires a working allocator, which is exactly what may have
    // just panicked, so we route through a fixed-size truncating
    // sink instead.
    let mut msg_buf = [0u8; stackchan_net::crash_latch::MSG_CAP];
    let msg_len = format_into(&mut msg_buf, format_args!("{}", info.message()));

    let (file_bytes, file_len, line) =
        info.location()
            .map_or(([0u8; stackchan_net::crash_latch::FILE_CAP], 0, 0), |loc| {
                let mut buf = [0u8; stackchan_net::crash_latch::FILE_CAP];
                let bytes = loc.file().as_bytes();
                let n = bytes.len().min(stackchan_net::crash_latch::FILE_CAP);
                buf[..n].copy_from_slice(&bytes[..n]);
                (buf, n, loc.line())
            });

    // SAFETY: `CRASH_LATCH` is `static mut`. The panic handler runs
    // with every other CPU activity suspended; `RECORDING`
    // serialises re-entrant calls.
    unsafe {
        let latch = &mut *core::ptr::addr_of_mut!(CRASH_LATCH);
        encode(latch, &msg_buf[..msg_len], &file_bytes[..file_len], line);
    }
}

/// Read [`CRASH_LATCH`], validate magic + checksum, and return the
/// decoded snapshot if both pass. Atomically zeroes the magic after
/// a successful read so the caller never sees the same crash twice.
///
/// # Safety
///
/// Touches the `static mut CRASH_LATCH`. Called once on boot before
/// any other task spawns; safe by single-threaded construction.
#[must_use]
pub fn consume_latch() -> Option<CrashSnapshot> {
    // SAFETY: called from `main` before any task spawn. No other
    // executor is running concurrently.
    unsafe {
        let latch = &mut *core::ptr::addr_of_mut!(CRASH_LATCH);
        let snap = decode(latch);
        if snap.is_some() {
            // Mark consumed so subsequent boots don't replay.
            clear_magic(latch);
        } else if u32::from_le_bytes([latch[0], latch[1], latch[2], latch[3]])
            == stackchan_net::crash_latch::MAGIC
        {
            // Magic survived but the checksum didn't — clear the
            // magic so we don't keep tripping on a corrupt latch.
            clear_magic(latch);
        }
        snap
    }
}

/// Render a [`CrashSnapshot`] into a human-readable, line-delimited
/// log entry suitable for writing to `/sd/CRASH.LOG`. Schema is
/// stable: `key=value` pairs, one per line, blank trailer.
///
/// Operators read this directly via `GET /crash` or by mounting the
/// SD card. Future versions may add fields (boot count, reset
/// reason, firmware version) — readers tolerate unknown keys.
///
/// Embedded carriage returns and newlines in the panic message are
/// escaped to `\r` / `\n` so each key occupies exactly one line —
/// otherwise a `panic!("a\nb")` splits the `message=` value across
/// two lines, and a `panic!("a\rb")` corrupts the line in any
/// terminal that interprets `\r` as a column reset.
#[must_use]
pub fn render_log_entry(snap: &CrashSnapshot, reset_reason: &str) -> alloc::string::String {
    use core::fmt::Write as _;
    let mut out = alloc::string::String::new();
    let _ = writeln!(out, "reset_reason={reset_reason}");
    let _ = writeln!(out, "file={}", snap.file_str());
    let _ = writeln!(out, "line={}", snap.line);
    let escaped = snap.message_str().replace('\r', r"\r").replace('\n', r"\n");
    let _ = writeln!(out, "message={escaped}");
    out
}
