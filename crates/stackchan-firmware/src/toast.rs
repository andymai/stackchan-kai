//! Toast overlay — short on-screen warn/error notifications.
//!
//! Opt-in via `BehaviorConfig::toast_overlay_enabled`. When enabled,
//! the render task paints the most recently pushed toast in a
//! horizontal band at the bottom of the LCD for [`TOAST_TTL_MS`]
//! after it was pushed. The slot is single-deep — a fresh push
//! overwrites the previous toast, so a burst of failures shows the
//! most recent rather than a cycling backlog.
//!
//! ## Why a separate slot instead of `Face::bubble`
//!
//! The bubble lives in `stackchan-core` and is part of the avatar's
//! emotional surface — set by skills, decorators, and the
//! soliloquy modifier. The toast is a *debug aid*: warning the
//! operator that something is wrong with the firmware itself, not
//! something the avatar is saying. Keeping it firmware-side means
//! it can fire from any task (Wi-Fi failure, panic recovery, BLE
//! bond eviction, audio underrun) without dragging a
//! firmware-specific notion into the host-side domain model.
//!
//! ## Tasks can push from any context
//!
//! `push` takes a critical-section lock on the slot and returns
//! immediately. It is safe to call from interrupts and async
//! contexts. The render task is the sole reader (via
//! [`current`]); it never blocks waiting for a toast.

use core::cell::Cell;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use stackchan_core::Instant;

/// How long a toast stays visible after being pushed, in ms. Long
/// enough to read a six-word message; short enough that a burst of
/// warnings doesn't keep occluding the lower-third of the face.
pub const TOAST_TTL_MS: u64 = 3_000;

/// Maximum byte length of a toast message. Sized for one line of
/// `FONT_10X20` across the 320-px framebuffer (10 px/char × 32 chars
/// = 320 px), the font the render path uses for the toast band.
pub const MAX_TOAST_LEN: usize = 32;

/// Severity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    /// Yellow band — non-fatal warning.
    Warn,
    /// Red band — error worth the operator's attention.
    Error,
}

/// One toast snapshot — what the renderer needs to paint a single
/// frame. Implements `PartialEq` so the render task can dirty-check
/// against the previous frame's toast cheaply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastDisplay {
    /// Severity tier.
    pub level: ToastLevel,
    /// Bounded-length message text.
    pub text: heapless::String<MAX_TOAST_LEN>,
    /// Wall-clock instant after which this toast stops being drawn.
    pub expires_at: Instant,
}

/// In-memory slot for the active toast, or `None` for the
/// no-toast steady state. Held in a critical-section mutex so any
/// task — including interrupt handlers — can push without yielding.
static SLOT: Mutex<CriticalSectionRawMutex, Cell<Option<ToastDisplay>>> =
    Mutex::new(Cell::new(None));

/// Push a toast. Overwrites any active toast — the render path
/// shows the most recent only.
pub fn push(level: ToastLevel, message: &str, now: Instant) {
    let mut text: heapless::String<MAX_TOAST_LEN> = heapless::String::new();
    for ch in message.chars() {
        if text.push(ch).is_err() {
            break;
        }
    }
    let display = ToastDisplay {
        level,
        text,
        expires_at: Instant::from_millis(now.as_millis() + TOAST_TTL_MS),
    };
    SLOT.lock(|cell| cell.set(Some(display)));
}

/// Push a toast using a `core::fmt::Write` builder.
///
/// Convenient when the caller has a `Debug2Format` value or wants to
/// compose a short structured message without allocating. Argument
/// order mirrors [`push`] (level, payload, timestamp).
pub fn push_fmt(level: ToastLevel, args: core::fmt::Arguments<'_>, now: Instant) {
    let mut buf: heapless::String<MAX_TOAST_LEN> = heapless::String::new();
    let _ = buf.write_fmt(args);
    let display = ToastDisplay {
        level,
        text: buf,
        expires_at: Instant::from_millis(now.as_millis() + TOAST_TTL_MS),
    };
    SLOT.lock(|cell| cell.set(Some(display)));
}

/// Read the active toast if one exists and `now` is before its
/// expiry. Returns `None` (and clears the slot) once the TTL has
/// passed so the next render skips the overlay.
#[must_use]
pub fn current(now: Instant) -> Option<ToastDisplay> {
    SLOT.lock(|cell| {
        let current = cell.take();
        match current {
            Some(display) if now < display.expires_at => {
                // Put it back; it's still live.
                cell.set(Some(display.clone()));
                Some(display)
            }
            _ => None,
        }
    })
}

/// Clear the slot. Test helper + a hook the operator dashboard can
/// call via `POST /toast/clear` if a future route exposes one.
pub fn clear() {
    SLOT.lock(|cell| cell.set(None));
}
