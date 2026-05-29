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
//! change — the public API of [`RuntimeState`] / `load` / `save`
//! stays the same.
//!
//! ## Wire format
//!
//! Tiny line-based key-value, one field per line:
//!
//! ```text
//! palette=cute
//! mood=playful
//! face_geometry=chibi
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
use stackchan_core::{FaceGeometry, Mood, Palette};

use crate::head::HeadOffsets;

/// In-memory snapshot of the runtime state. Defaults match the
/// avatar's neutral resting look so a brand-new device reads the
/// same state a missing-file fallback would produce.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RuntimeState {
    /// Current operator-selected palette. Mirrored into
    /// `entity.face.palette` on boot via the firmware's
    /// `PALETTE_SIGNAL`.
    pub palette: Palette,
    /// Current operator-selected mood baseline. Mirrored into
    /// `entity.mind.mood` on boot via the firmware's `MOOD_SIGNAL`.
    pub mood: Mood,
    /// Current operator-selected face geometry preset. Applied to
    /// `entity.face` on boot via the firmware's
    /// `FACE_GEOMETRY_SIGNAL`.
    pub face_geometry: FaceGeometry,
    /// Operator-supplied head zero-point correction. Persisted so a
    /// reboot doesn't reset a freshly-dialled offset; mirrored into
    /// [`crate::head::OFFSETS_SIGNAL`] + the [`crate::head::OFFSETS_CACHE`]
    /// at boot.
    pub head_offsets: HeadOffsets,
}

/// Render a [`RuntimeState`] to its line-based wire form.
#[must_use]
pub fn render(state: &RuntimeState) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "palette={}", state.palette.wire_str());
    let _ = writeln!(out, "mood={}", state.mood.wire_str());
    let _ = writeln!(out, "face_geometry={}", state.face_geometry.wire_str());
    // Two decimal places match the resolution operators dial via
    // `POST /head/offsets` (the JSON parser accepts any f32). Stored
    // separately so a future bump in precision doesn't fight the wire
    // format.
    let _ = writeln!(
        out,
        "head_yaw_offset={:.2}",
        state.head_offsets.yaw_offset_deg
    );
    let _ = writeln!(
        out,
        "head_tilt_offset={:.2}",
        state.head_offsets.tilt_offset_deg
    );
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
            "face_geometry" => {
                if let Some(g) = FaceGeometry::from_wire_str(value.trim()) {
                    out.face_geometry = g;
                }
            }
            "head_yaw_offset" => {
                if let Ok(v) = value.trim().parse::<f32>() {
                    out.head_offsets.yaw_offset_deg = v;
                }
            }
            "head_tilt_offset" => {
                if let Ok(v) = value.trim().parse::<f32>() {
                    out.head_offsets.tilt_offset_deg = v;
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
        face_geometry: FaceGeometry::Default,
        head_offsets: HeadOffsets {
            yaw_offset_deg: 0.0,
            tilt_offset_deg: 0.0,
        },
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
pub async fn load_into_cache(appearance: &stackchan_net::config::AppearanceConfig) -> RuntimeState {
    let raw = crate::storage::with_storage(crate::storage::FirmwareStorage::read_runtime).await;
    let state = match raw {
        Some(Ok(Some(text))) => parse(&text),
        Some(Ok(None)) => {
            defmt::info!(
                "runtime store: no /sd/RUNTIME.RON yet — seeding from boot config appearance"
            );
            seed_from_appearance(appearance)
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
            seed_from_appearance(appearance)
        }
    };
    set_cache(state);
    state
}

/// Build the first-boot runtime state from the boot config's pinned
/// appearance. Empty / unknown wire strings resolve to the variant
/// default — the same fallback [`parse`] applies to an unknown enum
/// value on disk. Only palette + `face_geometry` are pinnable; mood and
/// head offsets keep their defaults until an operator sets them.
fn seed_from_appearance(appearance: &stackchan_net::config::AppearanceConfig) -> RuntimeState {
    RuntimeState {
        palette: Palette::from_wire_str(&appearance.palette).unwrap_or_default(),
        face_geometry: FaceGeometry::from_wire_str(&appearance.face_geometry).unwrap_or_default(),
        ..RuntimeState::default()
    }
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

/// Update the face-geometry field and persist the cache. Same
/// semantics as [`update_palette`].
pub async fn update_face_geometry(geometry: FaceGeometry) -> bool {
    mutate(|s| s.face_geometry = geometry);
    persist().await
}

/// Update the head-offsets field and persist the cache. Same
/// semantics as [`update_palette`].
pub async fn update_head_offsets(offsets: HeadOffsets) -> bool {
    mutate(|s| s.head_offsets = offsets);
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
    use alloc::string::ToString;

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
            face_geometry: FaceGeometry::Chibi,
            head_offsets: HeadOffsets {
                yaw_offset_deg: 1.25,
                tilt_offset_deg: -3.5,
            },
        };
        assert_eq!(render_then_parse(&s), s);
    }

    #[test]
    fn parse_omitted_head_offsets_yields_zero() {
        // Older firmware that wrote only palette/mood/face_geometry
        // must still parse cleanly under a newer firmware that knows
        // about head_offsets.
        let input = "palette=cute\nmood=playful\nface_geometry=chibi\n";
        let s = parse(input);
        assert!(s.head_offsets.yaw_offset_deg.abs() < f32::EPSILON);
        assert!(s.head_offsets.tilt_offset_deg.abs() < f32::EPSILON);
    }

    #[test]
    fn parse_head_offsets_round_trips() {
        let input = "head_yaw_offset=2.50\nhead_tilt_offset=-1.75\n";
        let s = parse(input);
        assert!((s.head_offsets.yaw_offset_deg - 2.5).abs() < f32::EPSILON);
        assert!((s.head_offsets.tilt_offset_deg - (-1.75)).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_tolerates_blank_lines_and_extra_whitespace() {
        let input = "\n  palette = dark\n\n   mood=focus\n  face_geometry = wide \n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::Dark);
        assert_eq!(s.mood, Mood::Focus);
        assert_eq!(s.face_geometry, FaceGeometry::Wide);
    }

    #[test]
    fn parse_skips_unknown_keys() {
        let input = "palette=cute\nfuture_field=hello\nmood=playful\nface_geometry=sleepy\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::Cute);
        assert_eq!(s.mood, Mood::Playful);
        assert_eq!(s.face_geometry, FaceGeometry::Sleepy);
    }

    #[test]
    fn parse_falls_back_on_unknown_enum_value() {
        let input = "palette=rainbow\nmood=ecstatic\nface_geometry=compact\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::default());
        assert_eq!(s.mood, Mood::default());
        assert_eq!(s.face_geometry, FaceGeometry::default());
    }

    #[test]
    fn parse_omitted_face_geometry_yields_default() {
        // Older firmware that wrote palette + mood only must still
        // parse cleanly under a newer firmware that knows about
        // face_geometry.
        let input = "palette=cute\nmood=playful\n";
        let s = parse(input);
        assert_eq!(s.palette, Palette::Cute);
        assert_eq!(s.mood, Mood::Playful);
        assert_eq!(s.face_geometry, FaceGeometry::default());
    }

    #[test]
    fn parse_empty_yields_defaults() {
        let s = parse("");
        assert_eq!(s, RuntimeState::default());
    }

    #[test]
    fn seed_resolves_boot_appearance_wire_strings() {
        let appearance = stackchan_net::config::AppearanceConfig {
            palette: "cute".to_string(),
            face_geometry: "chibi".to_string(),
        };
        let seeded = seed_from_appearance(&appearance);
        assert_eq!(seeded.palette, Palette::Cute);
        assert_eq!(seeded.face_geometry, FaceGeometry::Chibi);
        // Unpinnable axes keep their defaults.
        assert_eq!(seeded.mood, Mood::default());
        assert_eq!(seeded.head_offsets, RuntimeState::default().head_offsets);
    }

    #[test]
    fn seed_falls_back_to_default_on_empty_or_unknown() {
        let empty = seed_from_appearance(&stackchan_net::config::AppearanceConfig::default());
        assert_eq!(empty, RuntimeState::default());
        let unknown = seed_from_appearance(&stackchan_net::config::AppearanceConfig {
            palette: "rainbow".to_string(),
            face_geometry: "compact".to_string(),
        });
        assert_eq!(unknown.palette, Palette::default());
        assert_eq!(unknown.face_geometry, FaceGeometry::default());
    }
}
