//! SD-backed runtime-state store for fast-changing operator selections.
//!
//! Tracks values that an operator flips frequently from the dashboard
//! or via MCP — palette, mood — and that should survive a reboot
//! without churning the hand-edited `STACKCHAN.RON` boot config.
//! Stored as `/sd/RUNTIME.RON` with the same atomic staging-file
//! writeback the boot config uses.
//!
//! ## Why SD-backed instead of NVS
//!
//! ESP-NOW NVS would require a partition-table-aware flash region;
//! getting the offset wrong risks colliding with the OTA partition
//! or coredump area. The SD card is already mounted, already
//! atomic-write-tested via `STACKCHAN.RON`, and already covered by
//! the offline-first fallback (the firmware boots fine without an
//! SD). A future swap to a real NVS backend is a backend-only
//! change — the public API of [`RuntimeState`] / [`load`] / [`save`]
//! stays the same.
//!
//! ## Wire format
//!
//! Tiny line-based key-value, one field per line:
//!
//! ```text
//! palette=cute
//! mood=playful
//! ```
//!
//! Trailing blank lines and unknown keys are tolerated (forward
//! compatibility — adding a new field doesn't brick a device that
//! reads back an older firmware's state). Unknown enum strings
//! revert to the variant default.

use alloc::string::String;
use core::cell::Cell;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use stackchan_core::{Mood, Palette};

/// In-memory snapshot of the runtime state. Defaults match the
/// avatar's neutral resting look so a brand-new device reads the
/// same state a missing-file fallback would produce.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeState {
    /// Current operator-selected palette. Mirrored into
    /// `entity.face.palette` on boot via the firmware's
    /// `PALETTE_SIGNAL`.
    pub palette: Palette,
    /// Current operator-selected mood baseline. Mirrored into
    /// `entity.mind.mood` on boot via the firmware's `MOOD_SIGNAL`.
    pub mood: Mood,
}

/// Render a [`RuntimeState`] to its line-based wire form.
#[must_use]
pub fn render(state: &RuntimeState) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "palette={}", state.palette.wire_str());
    let _ = writeln!(out, "mood={}", state.mood.wire_str());
    out
}

/// Parse a line-based wire form into a [`RuntimeState`].
///
/// Tolerant by design: unknown keys are skipped, missing keys fall
/// back to the variant default. A malformed file is therefore never
/// fatal — the avatar always boots with a sensible runtime state
/// even if the SD card holds garbage.
#[must_use]
pub fn parse(input: &str) -> RuntimeState {
    let mut out = RuntimeState::default();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        match key.trim() {
            "palette" => {
                if let Some(p) = Palette::from_wire_str(value.trim()) {
                    out.palette = p;
                }
            }
            "mood" => {
                if let Some(m) = Mood::from_wire_str(value.trim()) {
                    out.mood = m;
                }
            }
            _ => {}
        }
    }
    out
}

/// Convenience round-trip used by tests and as a sanity helper for
/// callers that want to validate a rendered string before writing.
#[cfg(test)]
fn render_then_parse(state: &RuntimeState) -> RuntimeState {
    parse(&render(state))
}

/// Cached current runtime state — single source of truth for what
/// gets persisted to `/sd/RUNTIME.RON`. The boot path seeds this
/// from disk; the HTTP / MCP handlers update it before kicking off
/// a writeback.
static CACHE: Mutex<CriticalSectionRawMutex, Cell<RuntimeState>> =
    Mutex::new(Cell::new(RuntimeState {
        palette: Palette::Default,
        mood: Mood::Neutral,
    }));

/// Snapshot the current cache.
#[must_use]
pub fn current() -> RuntimeState {
    CACHE.lock(Cell::get)
}

/// Replace the cache with `state`. Cheap; used by [`load_into_cache`]
/// at boot before the first SD write.
pub fn set_cache(state: RuntimeState) {
    CACHE.lock(|cell| cell.set(state));
}

/// Load `/sd/RUNTIME.RON` into the cache, falling back to defaults
/// on any error path.
///
/// Tolerant by design — missing file, decode failure, parse mismatch
/// all leave the cache at variant defaults; the avatar must boot
/// regardless of disk state. Returns the loaded (or default) state
/// so the caller can fan it out to `PALETTE_SIGNAL` / `MOOD_SIGNAL`
/// before the render task starts.
pub async fn load_into_cache() -> RuntimeState {
    let raw = crate::storage::with_storage(crate::storage::FirmwareStorage::read_runtime).await;
    let state = match raw {
        Some(Ok(Some(text))) => parse(&text),
        Some(Ok(None)) => {
            defmt::info!("runtime store: no /sd/RUNTIME.RON yet — using defaults");
            RuntimeState::default()
        }
        Some(Err(e)) => {
            defmt::warn!(
                "runtime store: read failed ({}); using defaults",
                defmt::Debug2Format(&e)
            );
            RuntimeState::default()
        }
        None => {
            defmt::info!("runtime store: no SD mounted — runtime state is non-persistent");
            RuntimeState::default()
        }
    };
    set_cache(state);
    state
}

/// Update the palette field and persist the cache.
///
/// Returns `true` on a successful disk write, `false` on any storage
/// failure (caller may surface a 500 — but the in-memory cache is
/// already updated, so the runtime change still takes effect; only
/// the across-reboot persistence is lost).
pub async fn update_palette(palette: Palette) -> bool {
    mutate(|s| s.palette = palette);
    persist().await
}

/// Update the mood field and persist the cache. Same semantics as
/// [`update_palette`].
pub async fn update_mood(mood: Mood) -> bool {
    mutate(|s| s.mood = mood);
    persist().await
}

/// Apply `f` to the cache atomically — read + modify + write happen
/// inside one critical section.
///
/// The previous shape — `current()` then `set_cache(...)` — left a
/// window where a second `update_*` could clobber the first axis
/// between the read and the set.
fn mutate(f: impl FnOnce(&mut RuntimeState)) {
    CACHE.lock(|cell| {
        let mut s = cell.get();
        f(&mut s);
        cell.set(s);
    });
}

/// Render the **current** cache and atomically replace
/// `/sd/RUNTIME.RON`.
///
/// Reading the cache inside the storage closure (rather than from a
/// caller-captured snapshot) means a second `update_*` that lands
/// between two persists doesn't get clobbered on disk: the second
/// SD write reads the merged state. Without this, an A-then-B
/// mutate sequence whose persists complete in B-then-A order would
/// drop B's field on disk.
async fn persist() -> bool {
    let outcome = crate::storage::with_storage(|s| {
        let rendered = render(&current());
        s.write_runtime(&rendered)
    })
    .await;
    match outcome {
        Some(Ok(())) => true,
        Some(Err(e)) => {
            defmt::warn!(
                "runtime store: write failed ({}); cache updated but not persisted",
                defmt::Debug2Format(&e)
            );
            false
        }
        None => {
            defmt::info!("runtime store: no SD mounted — change is RAM-only");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_round_trips() {
        let s = RuntimeState::default();
        assert_eq!(render_then_parse(&s), s);
    }

    #[test]
    fn non_default_state_round_trips() {
        let s = RuntimeState {
            palette: Palette::Cute,
            mood: Mood::Playful,
        };
        assert_eq!(render_then_parse(&s), s);
    }

    #[test]
    fn parse_tolerates_blank_lines_and_extra_whitespace() {
        let input = "\n  palette = dark\n\n   mood=focus\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::Dark);
        assert_eq!(s.mood, Mood::Focus);
    }

    #[test]
    fn parse_skips_unknown_keys() {
        let input = "palette=cute\nfuture_field=hello\nmood=playful\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::Cute);
        assert_eq!(s.mood, Mood::Playful);
    }

    #[test]
    fn parse_falls_back_on_unknown_enum_value() {
        let input = "palette=rainbow\nmood=ecstatic\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::default());
        assert_eq!(s.mood, Mood::default());
    }

    #[test]
    fn parse_empty_yields_defaults() {
        let s = parse("");
        assert_eq!(s, RuntimeState::default());
    }
}
