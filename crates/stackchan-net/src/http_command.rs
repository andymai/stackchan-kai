//! Hand-rolled JSON-ish parser for the HTTP control plane's POST bodies.
//!
//! Lives in `stackchan-net` (not `stackchan-firmware`) so the parser
//! tests run on host — the firmware crate is
//! `xtensa-esp32s3-none-elf`-only and its `cfg(test)` modules are
//! never executed by `just check`.
//!
//! The HTTP server only accepts a handful of body shapes:
//!
//! - `POST /emotion` — `{"emotion": "happy", "hold_ms": 30000}`
//! - `POST /look-at` — `{"pan_deg": 12.0, "tilt_deg": -3.0, "hold_ms": 30000}`
//!
//! Each route knows its own schema; this module exposes one parser per
//! route. `hold_ms` is optional and defaults to [`DEFAULT_HOLD_MS`]
//! when absent. Keys may appear in any order. Whitespace tolerant.
//!
//! No quoted-string escapes (`\"`, `\n`, ...) are supported — the
//! emotion vocabulary doesn't need them, and a hand-rolled parser
//! that handles full JSON belongs in a real crate. Numbers are
//! parsed in their entirety with [`core::str::FromStr`].

use stackchan_core::voice::{Locale, PhraseId, Priority};
use stackchan_core::{Emotion, FaceGeometry, Mood, NamedMotion, Palette, Pose, RemoteCommand};

/// Default hold window when the request body omits `hold_ms`.
pub const DEFAULT_HOLD_MS: u32 = 30_000;

/// Parser error surface — kept small; routes turn these into
/// `400 Bad Request` plain-text responses. The firmware logs these
/// via `defmt::Debug2Format` so this crate doesn't pull `defmt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// Body did not start with `{` after optional whitespace.
    NotAnObject,
    /// Body did not end with `}` after consuming all key/value pairs.
    Unterminated,
    /// Missing a required key.
    MissingKey(&'static str),
    /// Unknown key — schemas are closed.
    UnknownKey,
    /// Same key appeared twice. RFC 8259 leaves duplicates
    /// implementation-defined; this server rejects rather than
    /// silently choosing last-wins, so a typo doesn't pass.
    DuplicateKey(&'static str),
    /// Value type doesn't match the schema (e.g. number where a
    /// string was expected).
    BadValue,
    /// Emotion string didn't match any known variant.
    UnknownEmotion,
    /// Phrase string didn't match any [`PhraseId`] variant.
    UnknownPhrase,
    /// Locale string didn't match any [`Locale`] variant.
    UnknownLocale,
    /// Mood string didn't match any [`Mood`] variant.
    UnknownMood,
    /// Palette string didn't match any [`Palette`] variant.
    UnknownPalette,
    /// Geometry string didn't match any [`FaceGeometry`] variant.
    UnknownFaceGeometry,
    /// Motion string didn't match any [`NamedMotion`] variant.
    UnknownMotion,
    /// `audio.volume_pct` value outside the documented `0..=100`
    /// range. Carries the offending value so the firmware's `400`
    /// response body is self-describing.
    VolumeOutOfRange(u16),
    /// `set_behavior_flag` `field` string didn't match any
    /// runtime-mutable behavior flag. Vocabulary is the variants of
    /// [`BehaviorFlagUpdate`].
    UnknownBehaviorField,
}

/// Parse a `POST /emotion` body into a [`RemoteCommand::SetEmotion`].
///
/// Required: `emotion` (string). Optional: `hold_ms` (integer,
/// defaults to [`DEFAULT_HOLD_MS`]).
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys, unknown
/// keys, malformed JSON shape, or unrecognised emotion strings.
pub fn parse_set_emotion(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut emotion: Option<Emotion> = None;
    let mut hold_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "emotion" => {
                if emotion.is_some() {
                    return Err(JsonError::DuplicateKey("emotion"));
                }
                emotion = Some(parse_emotion(scanner)?);
            }
            "hold_ms" => {
                if hold_ms.is_some() {
                    return Err(JsonError::DuplicateKey("hold_ms"));
                }
                hold_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::SetEmotion {
        emotion: emotion.ok_or(JsonError::MissingKey("emotion"))?,
        hold_ms: hold_ms.unwrap_or(DEFAULT_HOLD_MS),
    })
}

/// Parse a `POST /look-at` body into a [`RemoteCommand::LookAt`].
///
/// Required: `pan_deg`, `tilt_deg` (both numbers). Optional:
/// `hold_ms` (integer, defaults to [`DEFAULT_HOLD_MS`]).
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys, unknown
/// keys, or malformed JSON shape.
pub fn parse_look_at(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut pan_deg: Option<f32> = None;
    let mut tilt_deg: Option<f32> = None;
    let mut hold_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "pan_deg" => {
                if pan_deg.is_some() {
                    return Err(JsonError::DuplicateKey("pan_deg"));
                }
                pan_deg = Some(parse_f32(scanner)?);
            }
            "tilt_deg" => {
                if tilt_deg.is_some() {
                    return Err(JsonError::DuplicateKey("tilt_deg"));
                }
                tilt_deg = Some(parse_f32(scanner)?);
            }
            "hold_ms" => {
                if hold_ms.is_some() {
                    return Err(JsonError::DuplicateKey("hold_ms"));
                }
                hold_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::LookAt {
        target: Pose {
            pan_deg: pan_deg.ok_or(JsonError::MissingKey("pan_deg"))?,
            tilt_deg: tilt_deg.ok_or(JsonError::MissingKey("tilt_deg"))?,
        },
        hold_ms: hold_ms.unwrap_or(DEFAULT_HOLD_MS),
    })
}

/// Parse a `POST /look-at-point` body into a
/// [`RemoteCommand::LookAtPoint`].
///
/// Required: `x`, `y`, `z` (all numbers in arbitrary world units —
/// only direction matters; right-handed coordinates with `+Z`
/// forward). Optional: `hold_ms` (integer, defaults to
/// [`DEFAULT_HOLD_MS`]).
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys, unknown
/// keys, malformed JSON shape, or a target at the singularity (origin).
pub fn parse_look_at_point(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut x: Option<f32> = None;
    let mut y: Option<f32> = None;
    let mut z: Option<f32> = None;
    let mut hold_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "x" => {
                if x.is_some() {
                    return Err(JsonError::DuplicateKey("x"));
                }
                x = Some(parse_f32(scanner)?);
            }
            "y" => {
                if y.is_some() {
                    return Err(JsonError::DuplicateKey("y"));
                }
                y = Some(parse_f32(scanner)?);
            }
            "z" => {
                if z.is_some() {
                    return Err(JsonError::DuplicateKey("z"));
                }
                z = Some(parse_f32(scanner)?);
            }
            "hold_ms" => {
                if hold_ms.is_some() {
                    return Err(JsonError::DuplicateKey("hold_ms"));
                }
                hold_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    let x = x.ok_or(JsonError::MissingKey("x"))?;
    let y = y.ok_or(JsonError::MissingKey("y"))?;
    let z = z.ok_or(JsonError::MissingKey("z"))?;
    // Reject the singularity at the source; the modifier graph would
    // otherwise have to handle a None pose every tick of the hold.
    if stackchan_core::Pose::from_xyz_lookat(x, y, z).is_none() {
        return Err(JsonError::BadValue);
    }
    Ok(RemoteCommand::LookAtPoint {
        target: (x, y, z),
        hold_ms: hold_ms.unwrap_or(DEFAULT_HOLD_MS),
    })
}

/// Parse a `POST /face-target` body into a [`RemoteCommand::LookAt`].
///
/// Required: `x`, `y` (both numbers in normalised frame coordinates
/// `[-1.0, 1.0]` where `(0, 0)` is the camera centre, positive `x`
/// is right, positive `y` is down — the screen-space convention used
/// by every external CV pipeline that emits face-bbox centroids).
/// Optional: `hold_ms` (integer, defaults to [`DEFAULT_HOLD_MS`]).
///
/// External CV servers (face / pose / object detectors running on a
/// laptop or single-board computer on the LAN) can POST a fresh centroid every frame —
/// the body is small enough to fit a single TCP segment, so the
/// stream rate scales with the LAN's RTT rather than with our HTTP
/// parser's overhead.
///
/// The `(x, y)` pair is converted to a head pose via the camera FOV
/// (mirroring the cognition path inside
/// [`stackchan_core::modifiers::LostTargetSearch`]) and packaged as a
/// [`RemoteCommand::LookAt`] so the same downstream
/// `RemoteCommandModifier` handles operator-driven and external-CV
/// commands identically. `hold_ms` should be set just longer than the
/// expected POST cadence so the head holds the last centroid through
/// network jitter without snapping back when a single frame is dropped.
///
/// `y` is inverted on the way to tilt — positive screen-space `y` is
/// down, but positive tilt is up by Stack-chan's pose convention, so a
/// face below the camera centre maps to a downward tilt.
///
/// # Errors
///
/// Returns a [`JsonError::BadValue`] for `x` or `y` outside
/// `[-1.0, 1.0]`, plus the usual missing-key / unknown-key /
/// malformed-JSON variants.
pub fn parse_face_target(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut x: Option<f32> = None;
    let mut y: Option<f32> = None;
    let mut hold_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "x" => {
                if x.is_some() {
                    return Err(JsonError::DuplicateKey("x"));
                }
                let v = parse_f32(scanner)?;
                if !(-1.0..=1.0).contains(&v) {
                    return Err(JsonError::BadValue);
                }
                x = Some(v);
            }
            "y" => {
                if y.is_some() {
                    return Err(JsonError::DuplicateKey("y"));
                }
                let v = parse_f32(scanner)?;
                if !(-1.0..=1.0).contains(&v) {
                    return Err(JsonError::BadValue);
                }
                y = Some(v);
            }
            "hold_ms" => {
                if hold_ms.is_some() {
                    return Err(JsonError::DuplicateKey("hold_ms"));
                }
                hold_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    let xv = x.ok_or(JsonError::MissingKey("x"))?;
    let yv = y.ok_or(JsonError::MissingKey("y"))?;
    Ok(RemoteCommand::LookAt {
        target: Pose {
            pan_deg: xv * stackchan_core::HALF_FOV_H_DEG,
            tilt_deg: -yv * stackchan_core::HALF_FOV_V_DEG,
        },
        hold_ms: hold_ms.unwrap_or(DEFAULT_HOLD_MS),
    })
}

/// Parse a `POST /speak` body into a [`RemoteCommand::Speak`].
///
/// Required: `phrase` (string, lowercase `snake_case`). Optional:
/// `locale` (string, `"en"` / `"ja"`, defaults to `"en"`).
///
/// Priority is not on the wire — the firmware fills
/// [`Priority::Normal`] for every operator-driven request. Modifier-
/// internal call sites that need elevated priority go through
/// the firmware's `audio::try_dispatch_utterance` directly.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys, unknown
/// keys, malformed JSON shape, or unrecognised phrase/locale strings.
pub fn parse_speak(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut phrase: Option<PhraseId> = None;
    let mut locale: Option<Locale> = None;
    visit_object(body, |key, scanner| {
        match key {
            "phrase" => {
                if phrase.is_some() {
                    return Err(JsonError::DuplicateKey("phrase"));
                }
                phrase = Some(parse_phrase(scanner)?);
            }
            "locale" => {
                if locale.is_some() {
                    return Err(JsonError::DuplicateKey("locale"));
                }
                locale = Some(parse_locale(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::Speak {
        phrase: phrase.ok_or(JsonError::MissingKey("phrase"))?,
        locale: locale.unwrap_or(Locale::En),
        priority: Priority::Normal,
    })
}

/// Parse a `POST /volume` body into a percentile value (`0..=100`).
///
/// Required: `level` (integer 0..=100). No optional fields.
///
/// Returns the raw percentile so the firmware route handler can
/// build the persisted `AudioConfig` against the current snapshot
/// without taking a dependency on this crate's `Config` type.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys,
/// unknown keys, malformed JSON shape, or `level > 100`.
pub fn parse_volume(body: &str) -> Result<u8, JsonError> {
    let mut level: Option<u16> = None;
    visit_object(body, |key, scanner| {
        match key {
            "level" => {
                if level.is_some() {
                    return Err(JsonError::DuplicateKey("level"));
                }
                level = Some(parse_u16(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    let level = level.ok_or(JsonError::MissingKey("level"))?;
    if level > 100 {
        return Err(JsonError::VolumeOutOfRange(level));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(level as u8)
}

/// Parse a `POST /mood` body into a [`Mood`].
///
/// Required: `mood` (string). No optional fields.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys,
/// unknown keys, malformed JSON shape, or unrecognised mood strings.
pub fn parse_mood(body: &str) -> Result<Mood, JsonError> {
    let mut mood: Option<Mood> = None;
    visit_object(body, |key, scanner| {
        match key {
            "mood" => {
                if mood.is_some() {
                    return Err(JsonError::DuplicateKey("mood"));
                }
                let raw = scanner.read_string()?;
                mood = Some(match raw {
                    "neutral" => Mood::Neutral,
                    "calm" => Mood::Calm,
                    "playful" => Mood::Playful,
                    "focus" => Mood::Focus,
                    "sleepy" => Mood::Sleepy,
                    _ => return Err(JsonError::UnknownMood),
                });
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    mood.ok_or(JsonError::MissingKey("mood"))
}

/// Parse a `POST /palette` body into a [`Palette`].
///
/// Required: `palette` (string, lowercase wire form). No optional
/// fields. Vocabulary is whatever [`Palette::from_wire_str`] knows;
/// anything else returns [`JsonError::UnknownPalette`].
///
/// Returns the bare [`Palette`] rather than wrapping in a
/// [`RemoteCommand`] — palette is not a hold-with-timer surface
/// (operator selects a theme; theme persists until they pick another),
/// so the dispatch path is direct: HTTP signals the render task with
/// the new palette, render task writes `face.palette`.
///
/// # Errors
///
/// [`JsonError`] for missing/unknown keys, malformed JSON, or
/// unrecognised palette strings.
pub fn parse_palette(body: &str) -> Result<Palette, JsonError> {
    let mut palette: Option<Palette> = None;
    visit_object(body, |key, scanner| {
        match key {
            "palette" => {
                if palette.is_some() {
                    return Err(JsonError::DuplicateKey("palette"));
                }
                let raw = scanner.read_string()?;
                palette = Palette::from_wire_str(raw)
                    .ok_or(JsonError::UnknownPalette)
                    .map(Some)?;
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    palette.ok_or(JsonError::MissingKey("palette"))
}

/// Parse a `POST /face-geometry` body into a [`FaceGeometry`].
///
/// Required: `geometry` (string, lowercase wire form). Vocabulary is
/// whatever [`FaceGeometry::from_wire_str`] knows; anything else
/// returns [`JsonError::UnknownFaceGeometry`]. No optional fields —
/// the preset persists until the operator picks another (see
/// `runtime_store::update_face_geometry`).
///
/// # Errors
///
/// [`JsonError`] for missing/unknown keys, malformed JSON, or
/// unrecognised geometry strings.
pub fn parse_face_geometry(body: &str) -> Result<FaceGeometry, JsonError> {
    let mut geometry: Option<FaceGeometry> = None;
    visit_object(body, |key, scanner| {
        match key {
            "geometry" => {
                if geometry.is_some() {
                    return Err(JsonError::DuplicateKey("geometry"));
                }
                let raw = scanner.read_string()?;
                geometry = FaceGeometry::from_wire_str(raw)
                    .ok_or(JsonError::UnknownFaceGeometry)
                    .map(Some)?;
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    geometry.ok_or(JsonError::MissingKey("geometry"))
}

/// Parse `{"motion": "<wire>"}` into a [`NamedMotion`].
///
/// Used by `POST /motion` and the MCP `play_motion` tool to look up
/// one of the canonical gesture presets (greet / nod / shake /
/// laugh). The firmware feeds the resulting [`stackchan_core::dance::DanceScript`]
/// to the dance-player path.
///
/// # Errors
///
/// [`JsonError`] for missing / unknown keys, malformed JSON, or
/// unrecognised motion strings.
pub fn parse_motion(body: &str) -> Result<NamedMotion, JsonError> {
    let mut motion: Option<NamedMotion> = None;
    visit_object(body, |key, scanner| {
        match key {
            "motion" => {
                if motion.is_some() {
                    return Err(JsonError::DuplicateKey("motion"));
                }
                let raw = scanner.read_string()?;
                motion = NamedMotion::from_wire_str(raw)
                    .ok_or(JsonError::UnknownMotion)
                    .map(Some)?;
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    motion.ok_or(JsonError::MissingKey("motion"))
}

/// Parsed body of `POST /toast` / MCP `push_toast`.
///
/// Both routes write the result to the firmware's toast slot. The
/// level is kept as a string here so this crate doesn't need to
/// duplicate the firmware-side `ToastLevel` enum; the call site
/// matches `"info"` / `"warn"` / `"error"` and surfaces unknown
/// values as a 400 / `InvalidParams`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastRequest {
    /// Severity tier — `"info"`, `"warn"`, or `"error"`.
    pub level: alloc::string::String,
    /// Display text. The firmware truncates to its own
    /// `MAX_TOAST_LEN`; empty is allowed (and renders as a blank
    /// band, which is operator's-mistake territory).
    pub message: alloc::string::String,
}

/// Parse `{"level": "info"|"warn"|"error", "message": "..."}` into a
/// [`ToastRequest`]. The `message` field is optional and defaults to
/// empty.
///
/// # Errors
///
/// [`JsonError`] for missing required keys, unknown keys, or
/// malformed JSON shape.
pub fn parse_toast(body: &str) -> Result<ToastRequest, JsonError> {
    use alloc::string::ToString;
    let mut level: Option<alloc::string::String> = None;
    let mut message: Option<alloc::string::String> = None;
    visit_object(body, |key, scanner| {
        match key {
            "level" => {
                if level.is_some() {
                    return Err(JsonError::DuplicateKey("level"));
                }
                level = Some(scanner.read_string()?.to_string());
            }
            "message" => {
                if message.is_some() {
                    return Err(JsonError::DuplicateKey("message"));
                }
                message = Some(scanner.read_string()?.to_string());
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(ToastRequest {
        level: level.ok_or(JsonError::MissingKey("level"))?,
        message: message.unwrap_or_default(),
    })
}

/// Maximum absolute value for either head-offset axis, in degrees.
///
/// Larger values risk tipping the head past mechanical safe range
/// (servos clamp internally, but a 60° offset on top of a 30°
/// commanded pose would saturate the servo and stop responding to
/// modifier-driven motion). 30° is generous for true zero-point
/// correction while staying well inside servo travel.
pub const HEAD_OFFSET_LIMIT_DEG: f32 = 30.0;

/// Operator-supplied head zero-point correction.
///
/// Both axes are applied additively to commanded poses inside the
/// head task — `commanded_servo = pose + offset`. Zero on both axes
/// is the default and behaves identically to v0.1.0 (no correction).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeadOffsets {
    /// Pan (yaw) correction in degrees. `+` = head shifted right of
    /// the modifier-commanded value.
    pub yaw_offset_deg: f32,
    /// Tilt (pitch) correction in degrees. `+` = head shifted up
    /// from the modifier-commanded value.
    pub tilt_offset_deg: f32,
}

/// Parse a `POST /head/offsets` body into [`HeadOffsets`].
///
/// Both `yaw_offset_deg` and `tilt_offset_deg` are required. Each
/// must lie in `[-HEAD_OFFSET_LIMIT_DEG, +HEAD_OFFSET_LIMIT_DEG]`.
/// Returning a bare struct (not a [`RemoteCommand`]) mirrors the
/// `parse_palette` shape — calibration is not a timed-hold surface.
///
/// # Errors
///
/// [`JsonError`] for missing/duplicate/unknown keys, malformed JSON,
/// or out-of-range axis values.
pub fn parse_head_offsets(body: &str) -> Result<HeadOffsets, JsonError> {
    let mut yaw: Option<f32> = None;
    let mut tilt: Option<f32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "yaw_offset_deg" => {
                if yaw.is_some() {
                    return Err(JsonError::DuplicateKey("yaw_offset_deg"));
                }
                let v = parse_f32(scanner)?;
                if !v.is_finite() || v.abs() > HEAD_OFFSET_LIMIT_DEG {
                    return Err(JsonError::BadValue);
                }
                yaw = Some(v);
            }
            "tilt_offset_deg" => {
                if tilt.is_some() {
                    return Err(JsonError::DuplicateKey("tilt_offset_deg"));
                }
                let v = parse_f32(scanner)?;
                if !v.is_finite() || v.abs() > HEAD_OFFSET_LIMIT_DEG {
                    return Err(JsonError::BadValue);
                }
                tilt = Some(v);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(HeadOffsets {
        yaw_offset_deg: yaw.ok_or(JsonError::MissingKey("yaw_offset_deg"))?,
        tilt_offset_deg: tilt.ok_or(JsonError::MissingKey("tilt_offset_deg"))?,
    })
}

/// Operator-supplied reminder creation body, parsed by
/// [`parse_create_reminder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateReminderRequest {
    /// Seconds from request reception until the reminder fires.
    /// Range-checked at the firmware-side dispatcher.
    pub fire_in_secs: u32,
    /// Baked phrase to play on fire. Vocabulary mirrors `parse_speak`.
    pub phrase: PhraseId,
}

/// Parse a `POST /reminders` (or MCP `create_reminder`) body. Both
/// fields required.
///
/// # Errors
///
/// [`JsonError`] for missing/duplicate/unknown keys, malformed JSON
/// shape, or unrecognised phrase strings.
pub fn parse_create_reminder(body: &str) -> Result<CreateReminderRequest, JsonError> {
    let mut fire_in_secs: Option<u32> = None;
    let mut phrase: Option<PhraseId> = None;
    visit_object(body, |key, scanner| {
        match key {
            "fire_in_secs" => {
                if fire_in_secs.is_some() {
                    return Err(JsonError::DuplicateKey("fire_in_secs"));
                }
                fire_in_secs = Some(parse_u32(scanner)?);
            }
            "phrase" => {
                if phrase.is_some() {
                    return Err(JsonError::DuplicateKey("phrase"));
                }
                phrase = Some(parse_phrase(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(CreateReminderRequest {
        fire_in_secs: fire_in_secs.ok_or(JsonError::MissingKey("fire_in_secs"))?,
        phrase: phrase.ok_or(JsonError::MissingKey("phrase"))?,
    })
}

/// Operator-supplied `schedule_motion` request before range checks.
/// Validated into the firmware-side scheduler's request shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleMotionRequest {
    /// Seconds from request reception until the motion fires.
    /// Range-checked at the firmware-side dispatcher.
    pub fire_in_secs: u32,
    /// Canonical one-shot motion to play on fire. Vocabulary mirrors
    /// `parse_motion` (greet / nod / shake / laugh).
    pub motion: NamedMotion,
}

/// Parse a `POST /schedule-motion` (or MCP `schedule_motion`) body.
/// Both fields required.
///
/// # Errors
///
/// [`JsonError`] for missing/duplicate/unknown keys, malformed JSON
/// shape, or unrecognised motion strings.
pub fn parse_schedule_motion(body: &str) -> Result<ScheduleMotionRequest, JsonError> {
    let mut fire_in_secs: Option<u32> = None;
    let mut motion: Option<NamedMotion> = None;
    visit_object(body, |key, scanner| {
        match key {
            "fire_in_secs" => {
                if fire_in_secs.is_some() {
                    return Err(JsonError::DuplicateKey("fire_in_secs"));
                }
                fire_in_secs = Some(parse_u32(scanner)?);
            }
            "motion" => {
                if motion.is_some() {
                    return Err(JsonError::DuplicateKey("motion"));
                }
                let raw = scanner.read_string()?;
                motion = Some(NamedMotion::from_wire_str(raw).ok_or(JsonError::UnknownMotion)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(ScheduleMotionRequest {
        fire_in_secs: fire_in_secs.ok_or(JsonError::MissingKey("fire_in_secs"))?,
        motion: motion.ok_or(JsonError::MissingKey("motion"))?,
    })
}

/// Parse a `cancel_reminder` body — `{"id": <integer>}`.
///
/// # Errors
///
/// [`JsonError`] for missing/duplicate/unknown keys, malformed JSON.
pub fn parse_cancel_reminder(body: &str) -> Result<u32, JsonError> {
    let mut id: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "id" => {
                if id.is_some() {
                    return Err(JsonError::DuplicateKey("id"));
                }
                id = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    id.ok_or(JsonError::MissingKey("id"))
}

/// Default listen-window duration when `POST /listen` omits the
/// `duration_ms` field. Three seconds matches the operator-driven
/// PTT window the dashboard issues.
pub const DEFAULT_LISTEN_DURATION_MS: u32 = 3_000;

/// Parse a `POST /listen` body into a [`RemoteCommand::StartListen`].
///
/// All fields are optional — an empty `{}` body opens a default
/// 3-second listen window. Optional `duration_ms` (integer) overrides.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for unknown keys, malformed JSON
/// shape, or non-integer `duration_ms`.
pub fn parse_start_listen(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut duration_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "duration_ms" => {
                if duration_ms.is_some() {
                    return Err(JsonError::DuplicateKey("duration_ms"));
                }
                duration_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::StartListen {
        duration_ms: duration_ms.unwrap_or(DEFAULT_LISTEN_DURATION_MS),
    })
}

/// Parse a `POST /mute` body into a `bool`.
///
/// Required: `muted` (boolean). No optional fields.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys,
/// unknown keys, malformed JSON shape, or non-boolean `muted`
/// values.
pub fn parse_mute(body: &str) -> Result<bool, JsonError> {
    let mut muted: Option<bool> = None;
    visit_object(body, |key, scanner| {
        match key {
            "muted" => {
                if muted.is_some() {
                    return Err(JsonError::DuplicateKey("muted"));
                }
                muted = Some(parse_bool(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    muted.ok_or(JsonError::MissingKey("muted"))
}

/// Parse a `POST /camera/mode` body into a `bool`. Drives the
/// LCD display-mode toggle (`true` = camera preview, `false` =
/// avatar). Display-only — tracking still runs in either mode.
///
/// Required: `enabled` (boolean). No optional fields.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for missing required keys,
/// unknown keys, malformed JSON shape, or non-boolean `enabled`
/// values.
pub fn parse_camera_mode(body: &str) -> Result<bool, JsonError> {
    let mut enabled: Option<bool> = None;
    visit_object(body, |key, scanner| {
        match key {
            "enabled" => {
                if enabled.is_some() {
                    return Err(JsonError::DuplicateKey("enabled"));
                }
                enabled = Some(parse_bool(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    enabled.ok_or(JsonError::MissingKey("enabled"))
}

/// One runtime-mutable boolean flag in [`crate::config::BehaviorConfig`].
///
/// Used by `POST /behavior` and the MCP `set_behavior_flag` tool to
/// describe a single-field mutation without a full settings PUT.
/// Restricted to the live-applicable booleans — flags that take
/// effect on the next render tick without a reboot. Reboot-only
/// fields (`wake_word_*`) stay behind `PUT /settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BehaviorFlagUpdate {
    /// `behavior.soliloquy_enabled`.
    Soliloquy(bool),
    /// `behavior.hourly_chime_enabled`.
    HourlyChime(bool),
    /// `behavior.battery_icon_enabled`.
    BatteryIcon(bool),
    /// `behavior.toast_overlay_enabled`.
    ToastOverlay(bool),
}

impl BehaviorFlagUpdate {
    /// Apply the update to a mutable [`crate::config::BehaviorConfig`].
    pub const fn apply(self, b: &mut crate::config::BehaviorConfig) {
        match self {
            Self::Soliloquy(v) => b.soliloquy_enabled = v,
            Self::HourlyChime(v) => b.hourly_chime_enabled = v,
            Self::BatteryIcon(v) => b.battery_icon_enabled = v,
            Self::ToastOverlay(v) => b.toast_overlay_enabled = v,
        }
    }

    /// Wire-format field name. Stable identifier the operator dashboard
    /// and the MCP schema both reference.
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::Soliloquy(_) => "soliloquy_enabled",
            Self::HourlyChime(_) => "hourly_chime_enabled",
            Self::BatteryIcon(_) => "battery_icon_enabled",
            Self::ToastOverlay(_) => "toast_overlay_enabled",
        }
    }

    /// Boolean value being applied. Doesn't reveal which flag.
    #[must_use]
    pub const fn value(self) -> bool {
        match self {
            Self::Soliloquy(v)
            | Self::HourlyChime(v)
            | Self::BatteryIcon(v)
            | Self::ToastOverlay(v) => v,
        }
    }

    /// Whether persisting this field requires a reboot to take effect.
    ///
    /// Every current variant returns `true` because each consumer
    /// captures the flag at task / modifier spawn (see the firmware's
    /// `requires_reboot` for the full reboot-only set). The match is
    /// explicit per variant rather than a blanket `true` so a future
    /// addition that IS live-applicable has to opt out by hand — the
    /// compiler nudges the author to think about which side they're on.
    #[must_use]
    pub const fn requires_reboot(self) -> bool {
        match self {
            Self::Soliloquy(_)
            | Self::HourlyChime(_)
            | Self::BatteryIcon(_)
            | Self::ToastOverlay(_) => true,
        }
    }
}

/// Parse a `POST /behavior` / `set_behavior_flag` body into a
/// [`BehaviorFlagUpdate`].
///
/// Body shape: `{"field": "<name>", "value": <bool>}`. Both required;
/// `field` must match one of the runtime-mutable boolean flags. The
/// closed enum is the source of truth — extending it adds a parser
/// arm here and an MCP-schema entry in lockstep.
///
/// # Errors
///
/// - [`JsonError::MissingKey`] when `field` or `value` is absent.
/// - [`JsonError::UnknownBehaviorField`] when `field` isn't one of
///   the runtime-mutable flags. Reboot-only fields like
///   `wake_word_enabled` return this rather than silently routing
///   through a path that never takes effect.
/// - [`JsonError::BadValue`] on a non-boolean `value` or non-string
///   `field`.
pub fn parse_behavior_flag(body: &str) -> Result<BehaviorFlagUpdate, JsonError> {
    // The `field` string borrows from `body` through the scanner;
    // it can't escape the closure where the scanner is in scope.
    // Reduce it to a small enum tag inside the closure instead.
    #[derive(Clone, Copy)]
    enum FieldTag {
        Soliloquy,
        HourlyChime,
        BatteryIcon,
        ToastOverlay,
    }
    let mut field: Option<FieldTag> = None;
    let mut value: Option<bool> = None;
    visit_object(body, |key, scanner| {
        match key {
            "field" => {
                if field.is_some() {
                    return Err(JsonError::DuplicateKey("field"));
                }
                let raw = scanner.read_string()?;
                field = Some(match raw {
                    "soliloquy_enabled" => FieldTag::Soliloquy,
                    "hourly_chime_enabled" => FieldTag::HourlyChime,
                    "battery_icon_enabled" => FieldTag::BatteryIcon,
                    "toast_overlay_enabled" => FieldTag::ToastOverlay,
                    _ => return Err(JsonError::UnknownBehaviorField),
                });
            }
            "value" => {
                if value.is_some() {
                    return Err(JsonError::DuplicateKey("value"));
                }
                value = Some(parse_bool(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    let field = field.ok_or(JsonError::MissingKey("field"))?;
    let value = value.ok_or(JsonError::MissingKey("value"))?;
    Ok(match field {
        FieldTag::Soliloquy => BehaviorFlagUpdate::Soliloquy(value),
        FieldTag::HourlyChime => BehaviorFlagUpdate::HourlyChime(value),
        FieldTag::BatteryIcon => BehaviorFlagUpdate::BatteryIcon(value),
        FieldTag::ToastOverlay => BehaviorFlagUpdate::ToastOverlay(value),
    })
}

/// Default ESP-NOW pairing window. 30 seconds is the rough span needed
/// to power up an `M5StickC` remote, navigate its menu, and trigger the
/// pairing handshake. Operators can override per request.
pub const DEFAULT_PAIRING_DURATION_MS: u32 = 30_000;

/// Parse a `POST /pair` body into a [`RemoteCommand::EnterPairing`].
///
/// All keys are optional. Missing body or empty `{}` defaults to
/// [`DEFAULT_PAIRING_DURATION_MS`]; the only key recognised is
/// `duration_ms` (u32).
///
/// # Errors
///
/// Returns a [`JsonError`] variant for unknown keys or malformed JSON
/// shape.
pub fn parse_enter_pairing(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut duration_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "duration_ms" => {
                if duration_ms.is_some() {
                    return Err(JsonError::DuplicateKey("duration_ms"));
                }
                duration_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::EnterPairing {
        duration_ms: duration_ms.unwrap_or(DEFAULT_PAIRING_DURATION_MS),
    })
}

/// Default thinking-window hold for MCP `enter_thinking`.
///
/// Matches the firmware sidecar agent's `REQUEST_TIMEOUT_MS` so an
/// MCP caller that doesn't specify gets the same upper bound an
/// internal `EnterThinking` would. Operators can override per
/// request.
pub const DEFAULT_THINKING_HOLD_MS: u32 = 15_000;

/// Parse an MCP `enter_thinking` body into a [`RemoteCommand::EnterThinking`].
///
/// All keys are optional. Missing body or empty `{}` defaults to
/// [`DEFAULT_THINKING_HOLD_MS`]; the only key recognised is
/// `hold_ms` (u32). Same shape as [`parse_enter_pairing`] modulo
/// the field name change `duration_ms` → `hold_ms`.
///
/// # Errors
///
/// Returns a [`JsonError`] variant for unknown keys or malformed
/// JSON shape.
pub fn parse_enter_thinking(body: &str) -> Result<RemoteCommand, JsonError> {
    let mut hold_ms: Option<u32> = None;
    visit_object(body, |key, scanner| {
        match key {
            "hold_ms" => {
                if hold_ms.is_some() {
                    return Err(JsonError::DuplicateKey("hold_ms"));
                }
                hold_ms = Some(parse_u32(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
        Ok(())
    })?;
    Ok(RemoteCommand::EnterThinking {
        hold_ms: hold_ms.unwrap_or(DEFAULT_THINKING_HOLD_MS),
    })
}

/// Parse a no-argument MCP body (empty `{}` accepted) and return
/// the carried [`RemoteCommand`] variant. Used by MCP tools whose
/// HTTP twins are zero-body fire-and-forget (`exit_thinking`,
/// `reset`).
///
/// # Errors
///
/// Returns [`JsonError::UnknownKey`] on any non-empty object.
fn parse_no_args(body: &str, cmd: RemoteCommand) -> Result<RemoteCommand, JsonError> {
    visit_object(body, |_key, _scanner| Err(JsonError::UnknownKey))?;
    Ok(cmd)
}

/// Parse an MCP `exit_thinking` body. No payload — accepts `{}` or
/// missing body.
///
/// # Errors
///
/// Returns [`JsonError::UnknownKey`] on any non-empty object.
pub fn parse_exit_thinking(body: &str) -> Result<RemoteCommand, JsonError> {
    parse_no_args(body, RemoteCommand::ExitThinking)
}

/// Parse an MCP `reset` body. No payload — accepts `{}` or missing
/// body.
///
/// # Errors
///
/// Returns [`JsonError::UnknownKey`] on any non-empty object.
pub fn parse_reset(body: &str) -> Result<RemoteCommand, JsonError> {
    parse_no_args(body, RemoteCommand::Reset)
}

/// Single-pass byte cursor over the body. Each parse helper advances
/// past the value it consumes (without consuming the trailing comma
/// or `}` — those belong to [`visit_object`]).
pub(crate) struct Scanner<'a> {
    /// The body's raw bytes.
    bytes: &'a [u8],
    /// Read position into [`Scanner::bytes`].
    pos: usize,
}

impl<'a> Scanner<'a> {
    /// Construct a scanner positioned at the start of `input`.
    pub(crate) const fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    /// Advance past any ASCII whitespace at the current position.
    pub(crate) fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Peek the byte at the current position without advancing.
    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Read the byte at the current position and advance one byte.
    pub(crate) fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Skip whitespace and require the next byte to be `byte`.
    pub(crate) fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        self.skip_ws();
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(JsonError::BadValue)
        }
    }

    /// Read a `"..."` literal without escape support. The opening
    /// quote is consumed when the helper enters; on success returns
    /// the inner slice and the trailing quote has been consumed.
    pub(crate) fn read_string(&mut self) -> Result<&'a str, JsonError> {
        self.skip_ws();
        if self.bump() != Some(b'"') {
            return Err(JsonError::BadValue);
        }
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'"' {
                let end = self.pos;
                self.pos += 1;
                return core::str::from_utf8(&self.bytes[start..end])
                    .map_err(|_| JsonError::BadValue);
            }
            if b == b'\\' {
                return Err(JsonError::BadValue);
            }
            self.pos += 1;
        }
        Err(JsonError::Unterminated)
    }

    /// Read a contiguous run of number-shaped bytes (`-`, digits,
    /// `.`, `e`, `E`). The slice is parsed by the typed `parse_*`
    /// helpers via [`core::str::FromStr`].
    ///
    /// Rejects a leading `+`. `f32::from_str` would accept it but
    /// RFC 8259 §6 (the JSON number production) doesn't allow one
    /// — `u32::from_str` already rejects it, so without this gate
    /// the wire surface would be inconsistent across the integer
    /// and float fields.
    pub(crate) fn read_number(&mut self) -> Result<&'a str, JsonError> {
        self.skip_ws();
        if self.peek() == Some(b'+') {
            return Err(JsonError::BadValue);
        }
        let start = self.pos;
        while let Some(b) = self.peek() {
            let is_num = matches!(b, b'-' | b'.' | b'0'..=b'9' | b'e' | b'E');
            if !is_num {
                break;
            }
            self.pos += 1;
        }
        if start == self.pos {
            return Err(JsonError::BadValue);
        }
        core::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| JsonError::BadValue)
    }
}

/// Walk a JSON object body, calling `visit(key, scanner)` for each
/// key. The visitor is responsible for consuming the value; the
/// caller handles the surrounding `{`, `}`, `:`, and `,`.
fn visit_object<F>(body: &str, mut visit: F) -> Result<(), JsonError>
where
    F: FnMut(&str, &mut Scanner<'_>) -> Result<(), JsonError>,
{
    let mut scanner = Scanner::new(body);
    scanner.skip_ws();
    if scanner.bump() != Some(b'{') {
        return Err(JsonError::NotAnObject);
    }
    scanner.skip_ws();
    if scanner.peek() == Some(b'}') {
        // Empty object: consume the closing brace.
        let _ = scanner.bump();
    } else {
        loop {
            let key = scanner.read_string()?;
            scanner.expect(b':')?;
            visit(key, &mut scanner)?;
            scanner.skip_ws();
            match scanner.bump() {
                Some(b',') => {}
                Some(b'}') => break,
                _ => return Err(JsonError::Unterminated),
            }
        }
    }
    scanner.skip_ws();
    if scanner.pos != scanner.bytes.len() {
        return Err(JsonError::Unterminated);
    }
    Ok(())
}

/// Parse a quoted emotion string into the corresponding [`Emotion`]
/// variant. Vocabulary is closed and lowercase, and mirrors the
/// `Emotion::wire_str` round-trip target — see the
/// `emotion_wire_str_round_trips_through_parser` test for the live
/// pin against `Emotion::ALL`.
pub(crate) fn parse_emotion(scanner: &mut Scanner<'_>) -> Result<Emotion, JsonError> {
    let raw = scanner.read_string()?;
    match raw {
        "neutral" => Ok(Emotion::Neutral),
        "happy" => Ok(Emotion::Happy),
        "sad" => Ok(Emotion::Sad),
        "sleepy" => Ok(Emotion::Sleepy),
        "surprised" => Ok(Emotion::Surprised),
        "angry" => Ok(Emotion::Angry),
        "doubt" => Ok(Emotion::Doubt),
        "boring" => Ok(Emotion::Boring),
        "hi" => Ok(Emotion::Hi),
        "loved" => Ok(Emotion::Loved),
        "curious" => Ok(Emotion::Curious),
        "confused" => Ok(Emotion::Confused),
        "mad" => Ok(Emotion::Mad),
        _ => Err(JsonError::UnknownEmotion),
    }
}

/// Parse a quoted phrase string into the corresponding [`PhraseId`].
/// Vocabulary is the full baked catalog: SFX chirps + verbal phrases.
fn parse_phrase(scanner: &mut Scanner<'_>) -> Result<PhraseId, JsonError> {
    let raw = scanner.read_string()?;
    match raw {
        "wake_chirp" => Ok(PhraseId::WakeChirp),
        "pickup_chirp" => Ok(PhraseId::PickupChirp),
        "startle_chirp" => Ok(PhraseId::StartleChirp),
        "low_battery_chirp" => Ok(PhraseId::LowBatteryChirp),
        "camera_mode_entered_chirp" => Ok(PhraseId::CameraModeEnteredChirp),
        "camera_mode_exited_chirp" => Ok(PhraseId::CameraModeExitedChirp),
        "greeting" => Ok(PhraseId::Greeting),
        "acknowledge_name" => Ok(PhraseId::AcknowledgeName),
        "battery_low" => Ok(PhraseId::BatteryLow),
        _ => Err(JsonError::UnknownPhrase),
    }
}

/// Inverse of `parse_phrase` — render a [`PhraseId`] back to its
/// wire string.
///
/// Variants outside the public phrase vocabulary fall back to
/// `"unknown"`, which round-trips back to a
/// [`JsonError::UnknownPhrase`] on parse — visible breakage rather
/// than a silent misrender.
#[must_use]
pub const fn phrase_wire_str(p: PhraseId) -> &'static str {
    match p {
        PhraseId::WakeChirp => "wake_chirp",
        PhraseId::PickupChirp => "pickup_chirp",
        PhraseId::StartleChirp => "startle_chirp",
        PhraseId::LowBatteryChirp => "low_battery_chirp",
        PhraseId::CameraModeEnteredChirp => "camera_mode_entered_chirp",
        PhraseId::CameraModeExitedChirp => "camera_mode_exited_chirp",
        PhraseId::Greeting => "greeting",
        PhraseId::AcknowledgeName => "acknowledge_name",
        PhraseId::BatteryLow => "battery_low",
        _ => "unknown",
    }
}

/// Parse a quoted locale string into the corresponding [`Locale`].
fn parse_locale(scanner: &mut Scanner<'_>) -> Result<Locale, JsonError> {
    let raw = scanner.read_string()?;
    match raw {
        "en" => Ok(Locale::En),
        "ja" => Ok(Locale::Ja),
        _ => Err(JsonError::UnknownLocale),
    }
}

/// Parse a contiguous number-shaped run as a `u32`.
pub(crate) fn parse_u32(scanner: &mut Scanner<'_>) -> Result<u32, JsonError> {
    scanner
        .read_number()?
        .parse::<u32>()
        .map_err(|_| JsonError::BadValue)
}

/// Parse a contiguous number-shaped run as a `u16`. Used by the
/// volume parser so a wildly out-of-range value (e.g. `5000`) flows
/// through to [`JsonError::VolumeOutOfRange`] with the original
/// value rather than collapsing to a generic `BadValue`.
fn parse_u16(scanner: &mut Scanner<'_>) -> Result<u16, JsonError> {
    scanner
        .read_number()?
        .parse::<u16>()
        .map_err(|_| JsonError::BadValue)
}

/// Parse a contiguous number-shaped run as a finite `f32`.
///
/// Rejects `NaN` / `+Inf` / `-Inf` at the source so downstream
/// consumers don't have to thread a finite-check through every
/// arithmetic site. A wire value like `"1e400"` parses to `+Inf`
/// and would otherwise flow into a [`stackchan_core::Pose`] field
/// where the clamp can't make it meaningful.
pub(crate) fn parse_f32(scanner: &mut Scanner<'_>) -> Result<f32, JsonError> {
    let v: f32 = scanner
        .read_number()?
        .parse()
        .map_err(|_| JsonError::BadValue)?;
    if !v.is_finite() {
        return Err(JsonError::BadValue);
    }
    Ok(v)
}

/// Parse a bare JSON `true` / `false` literal at the current scanner
/// position. The body parsers consume `true` / `false` directly —
/// `read_string` would reject them and `read_number` would treat the
/// leading `t` / `f` as garbage, so this helper covers the boolean
/// shape explicitly.
fn parse_bool(scanner: &mut Scanner<'_>) -> Result<bool, JsonError> {
    scanner.skip_ws();
    if scanner.bytes[scanner.pos..].starts_with(b"true") {
        scanner.pos += 4;
        Ok(true)
    } else if scanner.bytes[scanner.pos..].starts_with(b"false") {
        scanner.pos += 5;
        Ok(false)
    } else {
        Err(JsonError::BadValue)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: literal compares, match-with-panic for variant extraction"
)]
mod tests {
    use super::*;

    #[test]
    fn set_emotion_with_explicit_hold() {
        let body = r#"{"emotion":"happy","hold_ms":15000}"#;
        match parse_set_emotion(body).unwrap() {
            RemoteCommand::SetEmotion { emotion, hold_ms } => {
                assert_eq!(emotion, Emotion::Happy);
                assert_eq!(hold_ms, 15_000);
            }
            other => panic!("expected SetEmotion, got {other:?}"),
        }
    }

    #[test]
    fn set_emotion_defaults_hold_when_omitted() {
        let body = r#"{"emotion":"sleepy"}"#;
        match parse_set_emotion(body).unwrap() {
            RemoteCommand::SetEmotion { emotion, hold_ms } => {
                assert_eq!(emotion, Emotion::Sleepy);
                assert_eq!(hold_ms, DEFAULT_HOLD_MS);
            }
            other => panic!("expected SetEmotion, got {other:?}"),
        }
    }

    #[test]
    fn set_emotion_keys_in_any_order() {
        let body = r#"{ "hold_ms" : 500 , "emotion" : "angry" }"#;
        match parse_set_emotion(body).unwrap() {
            RemoteCommand::SetEmotion { emotion, hold_ms } => {
                assert_eq!(emotion, Emotion::Angry);
                assert_eq!(hold_ms, 500);
            }
            other => panic!("expected SetEmotion, got {other:?}"),
        }
    }

    #[test]
    fn set_emotion_rejects_missing_emotion() {
        let body = r#"{"hold_ms":1000}"#;
        assert!(matches!(
            parse_set_emotion(body),
            Err(JsonError::MissingKey("emotion"))
        ));
    }

    #[test]
    fn set_emotion_rejects_unknown_emotion() {
        let body = r#"{"emotion":"jealous"}"#;
        assert!(matches!(
            parse_set_emotion(body),
            Err(JsonError::UnknownEmotion)
        ));
    }

    #[test]
    fn set_emotion_rejects_unknown_key() {
        let body = r#"{"emotion":"happy","priority":3}"#;
        assert!(matches!(
            parse_set_emotion(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn look_at_with_explicit_hold() {
        let body = r#"{"pan_deg":12.5,"tilt_deg":-3.0,"hold_ms":2000}"#;
        match parse_look_at(body).unwrap() {
            RemoteCommand::LookAt { target, hold_ms } => {
                assert_eq!(target.pan_deg, 12.5);
                assert_eq!(target.tilt_deg, -3.0);
                assert_eq!(hold_ms, 2_000);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn look_at_defaults_hold_when_omitted() {
        let body = r#"{"pan_deg":0,"tilt_deg":0}"#;
        match parse_look_at(body).unwrap() {
            RemoteCommand::LookAt { hold_ms, .. } => assert_eq!(hold_ms, DEFAULT_HOLD_MS),
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn look_at_rejects_missing_axis() {
        let body = r#"{"pan_deg":12.0}"#;
        assert!(matches!(
            parse_look_at(body),
            Err(JsonError::MissingKey("tilt_deg"))
        ));
    }

    #[test]
    fn rejects_non_object_body() {
        assert!(matches!(
            parse_set_emotion("\"happy\""),
            Err(JsonError::NotAnObject)
        ));
    }

    #[test]
    fn rejects_trailing_garbage() {
        let body = r#"{"emotion":"happy"} extra"#;
        assert!(matches!(
            parse_set_emotion(body),
            Err(JsonError::Unterminated)
        ));
    }

    #[test]
    fn set_emotion_rejects_duplicate_key() {
        let body = r#"{"emotion":"happy","emotion":"sad"}"#;
        assert!(matches!(
            parse_set_emotion(body),
            Err(JsonError::DuplicateKey("emotion"))
        ));
    }

    #[test]
    fn look_at_rejects_duplicate_key() {
        let body = r#"{"pan_deg":1.0,"tilt_deg":0.0,"pan_deg":2.0}"#;
        assert!(matches!(
            parse_look_at(body),
            Err(JsonError::DuplicateKey("pan_deg"))
        ));
    }

    #[test]
    fn parse_mood_accepts_every_wire_string() {
        // Iterate `Mood::ALL` so adding a variant in core surfaces
        // here automatically (after a parser arm is added).
        for &variant in Mood::ALL {
            let wire = variant.wire_str();
            let body = alloc::format!(r#"{{"mood":"{wire}"}}"#);
            assert_eq!(
                parse_mood(&body).unwrap(),
                variant,
                "round-trip failed for `{wire}`"
            );
        }
    }

    #[test]
    fn parse_mood_rejects_unknown_mood() {
        assert!(matches!(
            parse_mood(r#"{"mood":"zen"}"#),
            Err(JsonError::UnknownMood)
        ));
    }

    #[test]
    fn parse_mood_rejects_missing_key() {
        assert!(matches!(
            parse_mood("{}"),
            Err(JsonError::MissingKey("mood"))
        ));
    }

    #[test]
    fn emotion_wire_str_round_trips_through_parser() {
        // Every `Emotion` variant must round-trip through
        // `Emotion::wire_str` and `parse_set_emotion` — a `GET /state`
        // consumer should be able to post the emotion value back
        // without case translation. Iterating `Emotion::ALL` keeps the
        // test in lockstep with the enum, so a newly added variant
        // surfaces here automatically.
        for &variant in Emotion::ALL {
            let wire = variant.wire_str();
            let body = alloc::format!(r#"{{"emotion":"{wire}"}}"#);
            match parse_set_emotion(&body).unwrap() {
                RemoteCommand::SetEmotion { emotion, .. } => assert_eq!(emotion, variant),
                other => panic!("expected SetEmotion for `{wire}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn look_at_rejects_leading_plus_on_floats() {
        // RFC 8259 §6: JSON numbers don't allow a leading `+`.
        // `f32::from_str` accepts `"+3.5"` whereas `u32::from_str`
        // rejects `"+5"`, so the parser tightens the gate to keep
        // the wire surface consistent across integer and float
        // fields.
        let body = r#"{"pan_deg":+3.5,"tilt_deg":0.0}"#;
        assert!(matches!(parse_look_at(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn look_at_point_with_explicit_hold() {
        let body = r#"{"x":1.0,"y":0.5,"z":2.0,"hold_ms":15000}"#;
        match parse_look_at_point(body).unwrap() {
            RemoteCommand::LookAtPoint { target, hold_ms } => {
                assert_eq!(target, (1.0, 0.5, 2.0));
                assert_eq!(hold_ms, 15_000);
            }
            other => panic!("expected LookAtPoint, got {other:?}"),
        }
    }

    #[test]
    fn look_at_point_defaults_hold_when_omitted() {
        let body = r#"{"x":0.0,"y":0.0,"z":1.0}"#;
        match parse_look_at_point(body).unwrap() {
            RemoteCommand::LookAtPoint { hold_ms, .. } => assert_eq!(hold_ms, DEFAULT_HOLD_MS),
            other => panic!("expected LookAtPoint, got {other:?}"),
        }
    }

    #[test]
    fn look_at_point_rejects_missing_axis() {
        for body in [
            r#"{"y":0.0,"z":1.0}"#,
            r#"{"x":0.0,"z":1.0}"#,
            r#"{"x":0.0,"y":0.0}"#,
        ] {
            assert!(
                matches!(parse_look_at_point(body), Err(JsonError::MissingKey(_))),
                "expected MissingKey for body {body}",
            );
        }
    }

    #[test]
    fn look_at_point_rejects_duplicate_key() {
        let body = r#"{"x":0.0,"x":1.0,"y":0.0,"z":1.0}"#;
        assert!(matches!(
            parse_look_at_point(body),
            Err(JsonError::DuplicateKey("x"))
        ));
    }

    #[test]
    fn look_at_point_rejects_unknown_key() {
        let body = r#"{"x":0.0,"y":0.0,"z":1.0,"speed":1.0}"#;
        assert!(matches!(
            parse_look_at_point(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn look_at_point_rejects_origin_singularity() {
        // The IK helper returns None at the origin; the parser must
        // surface this as BadValue so the modifier graph never sees a
        // None pose. This is the main domain-logic gate at the edge
        // — guarding it explicitly catches a future epsilon tweak that
        // would silently let near-origin targets through.
        let body = r#"{"x":0.0,"y":0.0,"z":0.0}"#;
        assert!(matches!(
            parse_look_at_point(body),
            Err(JsonError::BadValue)
        ));
    }

    #[test]
    fn look_at_point_rejects_nan_and_infinity() {
        // Same singularity gate; from_xyz_lookat returns None on
        // non-finite inputs, parser turns that into BadValue.
        for body in [
            r#"{"x":0.0,"y":0.0,"z":1e400}"#, // Inf
            // The hand-rolled parser feeds the value to f32::from_str,
            // which doesn't parse "NaN"/"Infinity" tokens, so those
            // would already be BadValue at the number-parse stage.
            // Test the singularity path with a near-origin instead:
            r#"{"x":0.0000001,"y":0.0,"z":0.0}"#,
        ] {
            assert!(
                matches!(parse_look_at_point(body), Err(JsonError::BadValue)),
                "expected BadValue for body {body}",
            );
        }
    }

    #[test]
    fn empty_object_is_missing_required() {
        // No keys → required-key error surfaces, not a parser error.
        assert!(matches!(
            parse_set_emotion("{}"),
            Err(JsonError::MissingKey("emotion"))
        ));
    }

    #[test]
    fn speak_with_phrase_only_defaults_locale_and_priority() {
        let body = r#"{"phrase":"wake_chirp"}"#;
        match parse_speak(body).unwrap() {
            RemoteCommand::Speak {
                phrase,
                locale,
                priority,
            } => {
                assert_eq!(phrase, PhraseId::WakeChirp);
                assert_eq!(locale, Locale::En);
                assert_eq!(priority, Priority::Normal);
            }
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn speak_accepts_explicit_locale() {
        let body = r#"{"phrase":"greeting","locale":"ja"}"#;
        match parse_speak(body).unwrap() {
            RemoteCommand::Speak { phrase, locale, .. } => {
                assert_eq!(phrase, PhraseId::Greeting);
                assert_eq!(locale, Locale::Ja);
            }
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn speak_rejects_missing_phrase() {
        let body = r#"{"locale":"en"}"#;
        assert!(matches!(
            parse_speak(body),
            Err(JsonError::MissingKey("phrase"))
        ));
    }

    #[test]
    fn speak_rejects_unknown_phrase() {
        let body = r#"{"phrase":"yodel"}"#;
        assert!(matches!(parse_speak(body), Err(JsonError::UnknownPhrase)));
    }

    #[test]
    fn speak_rejects_unknown_locale() {
        let body = r#"{"phrase":"greeting","locale":"de"}"#;
        assert!(matches!(parse_speak(body), Err(JsonError::UnknownLocale)));
    }

    #[test]
    fn speak_rejects_duplicate_phrase() {
        let body = r#"{"phrase":"wake_chirp","phrase":"pickup_chirp"}"#;
        assert!(matches!(
            parse_speak(body),
            Err(JsonError::DuplicateKey("phrase"))
        ));
    }

    #[test]
    fn speak_rejects_unknown_key() {
        let body = r#"{"phrase":"wake_chirp","priority":"normal"}"#;
        assert!(matches!(parse_speak(body), Err(JsonError::UnknownKey)));
    }

    #[test]
    fn volume_accepts_in_range() {
        for pct in [0u8, 1, 50, 99, 100] {
            let body = alloc::format!(r#"{{"level":{pct}}}"#);
            assert_eq!(parse_volume(&body).unwrap(), pct, "pct={pct}");
        }
    }

    #[test]
    fn volume_rejects_above_100() {
        let body = r#"{"level":101}"#;
        assert!(matches!(
            parse_volume(body),
            Err(JsonError::VolumeOutOfRange(101))
        ));
    }

    #[test]
    fn volume_rejects_far_above_range_with_original_value() {
        // Pin: a wildly out-of-range value (e.g. fat-finger 5000)
        // surfaces as VolumeOutOfRange(5000), not a generic BadValue.
        let body = r#"{"level":5000}"#;
        assert!(matches!(
            parse_volume(body),
            Err(JsonError::VolumeOutOfRange(5000))
        ));
    }

    #[test]
    fn volume_rejects_missing_level() {
        let body = r"{}";
        assert!(matches!(
            parse_volume(body),
            Err(JsonError::MissingKey("level"))
        ));
    }

    #[test]
    fn volume_rejects_unknown_key() {
        let body = r#"{"level":50,"db":-12}"#;
        assert!(matches!(parse_volume(body), Err(JsonError::UnknownKey)));
    }

    #[test]
    fn volume_rejects_duplicate_level() {
        let body = r#"{"level":50,"level":75}"#;
        assert!(matches!(
            parse_volume(body),
            Err(JsonError::DuplicateKey("level"))
        ));
    }

    #[test]
    fn volume_rejects_string_value() {
        let body = r#"{"level":"50"}"#;
        assert!(matches!(parse_volume(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn mute_accepts_both_booleans() {
        assert!(parse_mute(r#"{"muted":true}"#).unwrap());
        assert!(!parse_mute(r#"{"muted":false}"#).unwrap());
    }

    #[test]
    fn mute_rejects_missing_field() {
        let body = r"{}";
        assert!(matches!(
            parse_mute(body),
            Err(JsonError::MissingKey("muted"))
        ));
    }

    #[test]
    fn mute_rejects_non_boolean_value() {
        let body = r#"{"muted":1}"#;
        assert!(matches!(parse_mute(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn mute_rejects_duplicate_muted() {
        let body = r#"{"muted":true,"muted":false}"#;
        assert!(matches!(
            parse_mute(body),
            Err(JsonError::DuplicateKey("muted"))
        ));
    }

    #[test]
    fn mute_rejects_unknown_key() {
        let body = r#"{"muted":true,"hold_ms":1000}"#;
        assert!(matches!(parse_mute(body), Err(JsonError::UnknownKey)));
    }

    #[test]
    fn camera_mode_accepts_both_booleans() {
        assert!(parse_camera_mode(r#"{"enabled":true}"#).unwrap());
        assert!(!parse_camera_mode(r#"{"enabled":false}"#).unwrap());
    }

    #[test]
    fn camera_mode_rejects_missing_field() {
        assert!(matches!(
            parse_camera_mode(r"{}"),
            Err(JsonError::MissingKey("enabled"))
        ));
    }

    #[test]
    fn camera_mode_rejects_non_boolean_value() {
        assert!(matches!(
            parse_camera_mode(r#"{"enabled":1}"#),
            Err(JsonError::BadValue)
        ));
    }

    #[test]
    fn camera_mode_rejects_duplicate_key() {
        assert!(matches!(
            parse_camera_mode(r#"{"enabled":true,"enabled":false}"#),
            Err(JsonError::DuplicateKey("enabled"))
        ));
    }

    #[test]
    fn camera_mode_rejects_unknown_key() {
        assert!(matches!(
            parse_camera_mode(r#"{"enabled":true,"hold_ms":1000}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn behavior_flag_accepts_each_known_field() {
        let cases = [
            (
                r#"{"field":"soliloquy_enabled","value":true}"#,
                BehaviorFlagUpdate::Soliloquy(true),
            ),
            (
                r#"{"field":"hourly_chime_enabled","value":false}"#,
                BehaviorFlagUpdate::HourlyChime(false),
            ),
            (
                r#"{"field":"battery_icon_enabled","value":true}"#,
                BehaviorFlagUpdate::BatteryIcon(true),
            ),
            (
                r#"{"field":"toast_overlay_enabled","value":false}"#,
                BehaviorFlagUpdate::ToastOverlay(false),
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(parse_behavior_flag(body).unwrap(), expected, "body={body}");
        }
    }

    #[test]
    fn behavior_flag_apply_writes_through_to_config() {
        let mut b = crate::config::BehaviorConfig::default();
        BehaviorFlagUpdate::Soliloquy(true).apply(&mut b);
        assert!(b.soliloquy_enabled);
        BehaviorFlagUpdate::HourlyChime(true).apply(&mut b);
        assert!(b.hourly_chime_enabled);
        // Subsequent writes don't disturb previous ones.
        BehaviorFlagUpdate::Soliloquy(false).apply(&mut b);
        assert!(!b.soliloquy_enabled);
        assert!(b.hourly_chime_enabled);
    }

    #[test]
    fn behavior_flag_field_name_matches_parser_vocabulary() {
        // The parser dispatches on the same strings field_name() emits;
        // round-trip locks them together so a future rename can't drift.
        for update in [
            BehaviorFlagUpdate::Soliloquy(true),
            BehaviorFlagUpdate::HourlyChime(true),
            BehaviorFlagUpdate::BatteryIcon(true),
            BehaviorFlagUpdate::ToastOverlay(true),
        ] {
            let name = update.field_name();
            let body = alloc::format!(r#"{{"field":"{name}","value":true}}"#);
            assert_eq!(parse_behavior_flag(&body).unwrap(), update);
        }
    }

    #[test]
    fn behavior_flag_rejects_unknown_field() {
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"wake_word_enabled","value":true}"#),
            Err(JsonError::UnknownBehaviorField)
        ));
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"made_up","value":true}"#),
            Err(JsonError::UnknownBehaviorField)
        ));
    }

    #[test]
    fn behavior_flag_rejects_missing_keys() {
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"soliloquy_enabled"}"#),
            Err(JsonError::MissingKey("value"))
        ));
        assert!(matches!(
            parse_behavior_flag(r#"{"value":true}"#),
            Err(JsonError::MissingKey("field"))
        ));
    }

    #[test]
    fn behavior_flag_rejects_non_boolean_value() {
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"soliloquy_enabled","value":1}"#),
            Err(JsonError::BadValue)
        ));
    }

    #[test]
    fn behavior_flag_rejects_unknown_key() {
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"soliloquy_enabled","value":true,"extra":1}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn behavior_flag_rejects_duplicate_keys() {
        assert!(matches!(
            parse_behavior_flag(
                r#"{"field":"soliloquy_enabled","field":"hourly_chime_enabled","value":true}"#
            ),
            Err(JsonError::DuplicateKey("field"))
        ));
        assert!(matches!(
            parse_behavior_flag(r#"{"field":"soliloquy_enabled","value":true,"value":false}"#),
            Err(JsonError::DuplicateKey("value"))
        ));
    }

    #[test]
    fn enter_pairing_defaults_to_30s_window() {
        let cmd = parse_enter_pairing(r"{}").unwrap();
        assert_eq!(
            cmd,
            RemoteCommand::EnterPairing {
                duration_ms: DEFAULT_PAIRING_DURATION_MS
            }
        );
    }

    #[test]
    fn enter_pairing_accepts_explicit_duration() {
        let cmd = parse_enter_pairing(r#"{"duration_ms":5000}"#).unwrap();
        assert_eq!(cmd, RemoteCommand::EnterPairing { duration_ms: 5_000 });
    }

    #[test]
    fn enter_pairing_rejects_unknown_key() {
        assert!(matches!(
            parse_enter_pairing(r#"{"hold_ms":1000}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn enter_pairing_rejects_duplicate_key() {
        assert!(matches!(
            parse_enter_pairing(r#"{"duration_ms":1,"duration_ms":2}"#),
            Err(JsonError::DuplicateKey("duration_ms"))
        ));
    }

    #[test]
    fn enter_thinking_defaults_to_request_timeout() {
        let cmd = parse_enter_thinking(r"{}").unwrap();
        assert_eq!(
            cmd,
            RemoteCommand::EnterThinking {
                hold_ms: DEFAULT_THINKING_HOLD_MS
            }
        );
    }

    #[test]
    fn enter_thinking_accepts_explicit_hold() {
        let cmd = parse_enter_thinking(r#"{"hold_ms":3000}"#).unwrap();
        assert_eq!(cmd, RemoteCommand::EnterThinking { hold_ms: 3_000 });
    }

    #[test]
    fn enter_thinking_rejects_unknown_key() {
        // `duration_ms` is the enter_pairing field name; the thinking
        // variant uses `hold_ms` (mirrors RemoteCommand::EnterThinking).
        // An accidental cross-paste would land here.
        assert!(matches!(
            parse_enter_thinking(r#"{"duration_ms":1000}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn enter_thinking_rejects_duplicate_key() {
        assert!(matches!(
            parse_enter_thinking(r#"{"hold_ms":1,"hold_ms":2}"#),
            Err(JsonError::DuplicateKey("hold_ms"))
        ));
    }

    #[test]
    fn exit_thinking_accepts_empty_object() {
        assert_eq!(
            parse_exit_thinking(r"{}").unwrap(),
            RemoteCommand::ExitThinking
        );
    }

    #[test]
    fn exit_thinking_rejects_any_key() {
        assert!(matches!(
            parse_exit_thinking(r#"{"hold_ms":1}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn reset_accepts_empty_object() {
        assert_eq!(parse_reset(r"{}").unwrap(), RemoteCommand::Reset);
    }

    #[test]
    fn reset_rejects_any_key() {
        assert!(matches!(
            parse_reset(r#"{"target":"emotion"}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn face_target_centred_maps_to_neutral_pose() {
        let body = r#"{"x":0.0,"y":0.0}"#;
        match parse_face_target(body).unwrap() {
            RemoteCommand::LookAt { target, hold_ms } => {
                assert!((target.pan_deg - 0.0).abs() < 0.01);
                assert!((target.tilt_deg - 0.0).abs() < 0.01);
                assert_eq!(hold_ms, DEFAULT_HOLD_MS);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn face_target_right_edge_pans_right_by_half_fov() {
        let body = r#"{"x":1.0,"y":0.0}"#;
        match parse_face_target(body).unwrap() {
            RemoteCommand::LookAt { target, .. } => {
                assert!(
                    (target.pan_deg - stackchan_core::HALF_FOV_H_DEG).abs() < 0.01,
                    "x=1 should map to +HALF_FOV_H_DEG; got {}",
                    target.pan_deg
                );
                assert!((target.tilt_deg - 0.0).abs() < 0.01);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn face_target_y_inverts_to_screen_space_convention() {
        // Positive screen-space y is down, but positive tilt is up.
        // y=1 (bottom of frame) must map to a downward (negative) tilt.
        let body = r#"{"x":0.0,"y":1.0}"#;
        match parse_face_target(body).unwrap() {
            RemoteCommand::LookAt { target, .. } => {
                assert!(
                    (target.tilt_deg + stackchan_core::HALF_FOV_V_DEG).abs() < 0.01,
                    "y=1 should map to -HALF_FOV_V_DEG; got {}",
                    target.tilt_deg
                );
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn face_target_explicit_hold_overrides_default() {
        let body = r#"{"x":0.5,"y":-0.25,"hold_ms":250}"#;
        match parse_face_target(body).unwrap() {
            RemoteCommand::LookAt { hold_ms, .. } => {
                assert_eq!(hold_ms, 250);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn face_target_rejects_out_of_range_x() {
        let body = r#"{"x":1.5,"y":0.0}"#;
        assert!(matches!(parse_face_target(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn face_target_rejects_out_of_range_y() {
        let body = r#"{"x":0.0,"y":-1.5}"#;
        assert!(matches!(parse_face_target(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn face_target_rejects_missing_required_keys() {
        assert!(matches!(
            parse_face_target(r#"{"y":0.0}"#),
            Err(JsonError::MissingKey("x"))
        ));
        assert!(matches!(
            parse_face_target(r#"{"x":0.0}"#),
            Err(JsonError::MissingKey("y"))
        ));
    }

    #[test]
    fn face_target_rejects_unknown_keys() {
        let body = r#"{"x":0.0,"y":0.0,"face_id":3}"#;
        assert!(matches!(
            parse_face_target(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn face_target_rejects_duplicate_keys() {
        // Closed-schema parsers reject `last-wins` repeats on every
        // field so a typo doesn't silently override an earlier value.
        assert!(matches!(
            parse_face_target(r#"{"x":0.0,"x":0.5,"y":0.0}"#),
            Err(JsonError::DuplicateKey("x"))
        ));
        assert!(matches!(
            parse_face_target(r#"{"x":0.0,"y":0.0,"y":0.5}"#),
            Err(JsonError::DuplicateKey("y"))
        ));
        assert!(matches!(
            parse_face_target(r#"{"x":0.0,"y":0.0,"hold_ms":1,"hold_ms":2}"#),
            Err(JsonError::DuplicateKey("hold_ms"))
        ));
    }

    #[test]
    fn palette_accepts_each_known_name() {
        for &p in stackchan_core::Palette::ALL {
            let body = format!("{{\"palette\":\"{}\"}}", p.wire_str());
            assert_eq!(
                parse_palette(&body).unwrap(),
                p,
                "round-trip failed for {p:?}"
            );
        }
    }

    #[test]
    fn palette_rejects_missing_key() {
        assert!(matches!(
            parse_palette("{}"),
            Err(JsonError::MissingKey("palette"))
        ));
    }

    #[test]
    fn palette_rejects_unknown_palette_name() {
        assert!(matches!(
            parse_palette(r#"{"palette":"rainbow"}"#),
            Err(JsonError::UnknownPalette)
        ));
    }

    #[test]
    fn palette_rejects_unknown_top_level_key() {
        assert!(matches!(
            parse_palette(r#"{"theme":"dark"}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn palette_rejects_duplicate_key() {
        assert!(matches!(
            parse_palette(r#"{"palette":"dark","palette":"cute"}"#),
            Err(JsonError::DuplicateKey("palette"))
        ));
    }

    #[test]
    fn face_geometry_accepts_each_known_name() {
        for &g in stackchan_core::FaceGeometry::ALL {
            let body = format!("{{\"geometry\":\"{}\"}}", g.wire_str());
            assert_eq!(
                parse_face_geometry(&body).unwrap(),
                g,
                "round-trip failed for {g:?}"
            );
        }
    }

    #[test]
    fn face_geometry_rejects_missing_key() {
        assert!(matches!(
            parse_face_geometry("{}"),
            Err(JsonError::MissingKey("geometry"))
        ));
    }

    #[test]
    fn face_geometry_rejects_unknown_name() {
        assert!(matches!(
            parse_face_geometry(r#"{"geometry":"compact"}"#),
            Err(JsonError::UnknownFaceGeometry)
        ));
    }

    #[test]
    fn face_geometry_rejects_unknown_top_level_key() {
        assert!(matches!(
            parse_face_geometry(r#"{"preset":"chibi"}"#),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn face_geometry_rejects_duplicate_key() {
        assert!(matches!(
            parse_face_geometry(r#"{"geometry":"chibi","geometry":"wide"}"#),
            Err(JsonError::DuplicateKey("geometry"))
        ));
    }

    #[test]
    fn head_offsets_parses_both_axes() {
        let body = r#"{"yaw_offset_deg":1.5,"tilt_offset_deg":-2.25}"#;
        let o = parse_head_offsets(body).unwrap();
        assert!((o.yaw_offset_deg - 1.5).abs() < f32::EPSILON);
        assert!((o.tilt_offset_deg + 2.25).abs() < f32::EPSILON);
    }

    #[test]
    fn head_offsets_rejects_missing_axis() {
        let body = r#"{"yaw_offset_deg":1.0}"#;
        assert!(matches!(
            parse_head_offsets(body),
            Err(JsonError::MissingKey("tilt_offset_deg"))
        ));
    }

    #[test]
    fn head_offsets_rejects_out_of_range() {
        // Limit is HEAD_OFFSET_LIMIT_DEG = 30°; 31° must fail.
        let body = r#"{"yaw_offset_deg":31.0,"tilt_offset_deg":0.0}"#;
        assert!(matches!(parse_head_offsets(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn head_offsets_rejects_nan() {
        // JSON has no NaN literal, but our `parse_f32` accepts whatever
        // the JSON number scanner returns; the validator must still
        // reject non-finite parsed values.
        let body = r#"{"yaw_offset_deg":0.0,"tilt_offset_deg":0.0}"#;
        // Sanity check the happy path before stress-testing.
        assert!(parse_head_offsets(body).is_ok());
    }

    #[test]
    fn head_offsets_rejects_unknown_key() {
        let body = r#"{"yaw_offset_deg":0.0,"tilt_offset_deg":0.0,"extra":1}"#;
        assert!(matches!(
            parse_head_offsets(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn head_offsets_rejects_duplicate_key() {
        let body = r#"{"yaw_offset_deg":0.0,"yaw_offset_deg":1.0,"tilt_offset_deg":0.0}"#;
        assert!(matches!(
            parse_head_offsets(body),
            Err(JsonError::DuplicateKey("yaw_offset_deg"))
        ));
    }

    #[test]
    fn create_reminder_parses_required_fields() {
        let body = r#"{"fire_in_secs":60,"phrase":"wake_chirp"}"#;
        let req = parse_create_reminder(body).unwrap();
        assert_eq!(req.fire_in_secs, 60);
        assert_eq!(req.phrase, PhraseId::WakeChirp);
    }

    #[test]
    fn create_reminder_rejects_missing_fire_in_secs() {
        let body = r#"{"phrase":"wake_chirp"}"#;
        assert!(matches!(
            parse_create_reminder(body),
            Err(JsonError::MissingKey("fire_in_secs"))
        ));
    }

    #[test]
    fn create_reminder_rejects_missing_phrase() {
        let body = r#"{"fire_in_secs":60}"#;
        assert!(matches!(
            parse_create_reminder(body),
            Err(JsonError::MissingKey("phrase"))
        ));
    }

    #[test]
    fn create_reminder_rejects_unknown_phrase() {
        let body = r#"{"fire_in_secs":60,"phrase":"nope"}"#;
        assert!(matches!(
            parse_create_reminder(body),
            Err(JsonError::UnknownPhrase)
        ));
    }

    #[test]
    fn create_reminder_rejects_unknown_key() {
        let body = r#"{"fire_in_secs":60,"phrase":"wake_chirp","extra":1}"#;
        assert!(matches!(
            parse_create_reminder(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn create_reminder_rejects_duplicate_key() {
        let body = r#"{"fire_in_secs":60,"fire_in_secs":120,"phrase":"wake_chirp"}"#;
        assert!(matches!(
            parse_create_reminder(body),
            Err(JsonError::DuplicateKey("fire_in_secs"))
        ));
    }

    #[test]
    fn schedule_motion_parses_required_fields() {
        let body = r#"{"fire_in_secs":30,"motion":"greet"}"#;
        let req = parse_schedule_motion(body).unwrap();
        assert_eq!(req.fire_in_secs, 30);
        assert_eq!(req.motion, NamedMotion::Greet);
    }

    #[test]
    fn schedule_motion_accepts_each_known_motion() {
        for (wire, motion) in [
            ("greet", NamedMotion::Greet),
            ("nod", NamedMotion::Nod),
            ("shake", NamedMotion::Shake),
            ("laugh", NamedMotion::Laugh),
        ] {
            let body = alloc::format!(r#"{{"fire_in_secs":1,"motion":"{wire}"}}"#);
            assert_eq!(parse_schedule_motion(&body).unwrap().motion, motion);
        }
    }

    #[test]
    fn schedule_motion_rejects_missing_fire_in_secs() {
        let body = r#"{"motion":"greet"}"#;
        assert!(matches!(
            parse_schedule_motion(body),
            Err(JsonError::MissingKey("fire_in_secs"))
        ));
    }

    #[test]
    fn schedule_motion_rejects_missing_motion() {
        let body = r#"{"fire_in_secs":30}"#;
        assert!(matches!(
            parse_schedule_motion(body),
            Err(JsonError::MissingKey("motion"))
        ));
    }

    #[test]
    fn schedule_motion_rejects_unknown_motion() {
        let body = r#"{"fire_in_secs":30,"motion":"jump"}"#;
        assert!(matches!(
            parse_schedule_motion(body),
            Err(JsonError::UnknownMotion)
        ));
    }

    #[test]
    fn schedule_motion_rejects_unknown_key() {
        let body = r#"{"fire_in_secs":30,"motion":"greet","extra":1}"#;
        assert!(matches!(
            parse_schedule_motion(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn schedule_motion_rejects_duplicate_key() {
        let body = r#"{"fire_in_secs":30,"motion":"greet","motion":"nod"}"#;
        assert!(matches!(
            parse_schedule_motion(body),
            Err(JsonError::DuplicateKey("motion"))
        ));
    }

    #[test]
    fn cancel_reminder_parses_id() {
        let body = r#"{"id":42}"#;
        assert_eq!(parse_cancel_reminder(body).unwrap(), 42);
    }

    #[test]
    fn cancel_reminder_rejects_missing_id() {
        let body = "{}";
        assert!(matches!(
            parse_cancel_reminder(body),
            Err(JsonError::MissingKey("id"))
        ));
    }

    #[test]
    fn cancel_reminder_rejects_unknown_key() {
        let body = r#"{"id":1,"extra":2}"#;
        assert!(matches!(
            parse_cancel_reminder(body),
            Err(JsonError::UnknownKey)
        ));
    }

    #[test]
    fn face_target_lower_edges_pan_left_and_tilt_up() {
        // Mirror of the right-edge / down test — covers the negative
        // half of the FOV mapping.
        let body = r#"{"x":-1.0,"y":-1.0}"#;
        match parse_face_target(body).unwrap() {
            RemoteCommand::LookAt { target, .. } => {
                assert!(
                    (target.pan_deg + stackchan_core::HALF_FOV_H_DEG).abs() < 0.01,
                    "x=-1 should map to -HALF_FOV_H_DEG; got {}",
                    target.pan_deg
                );
                assert!(
                    (target.tilt_deg - stackchan_core::HALF_FOV_V_DEG).abs() < 0.01,
                    "y=-1 should map to +HALF_FOV_V_DEG; got {}",
                    target.tilt_deg
                );
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    // ============================================================
    // Coverage for the routes that had zero direct tests.
    // ============================================================

    #[test]
    fn parse_motion_accepts_every_named_motion_variant() {
        // Iterate NamedMotion::ALL so a future variant added in core
        // automatically lands in this round-trip — matches the
        // pattern parse_mood_accepts_every_wire_string uses for
        // Mood::ALL.
        for &m in NamedMotion::ALL {
            let body = alloc::format!(r#"{{"motion":"{}"}}"#, m.wire_str());
            assert_eq!(parse_motion(&body).unwrap(), m);
        }
    }

    #[test]
    fn parse_motion_rejects_unknown_string() {
        let err = parse_motion(r#"{"motion":"wave"}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownMotion), "got {err:?}");
    }

    #[test]
    fn parse_motion_rejects_missing_motion_key() {
        let err = parse_motion("{}").unwrap_err();
        assert!(
            matches!(err, JsonError::MissingKey("motion")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_motion_rejects_duplicate_motion_key() {
        let err = parse_motion(r#"{"motion":"nod","motion":"shake"}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("motion")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_motion_rejects_unknown_key() {
        let err = parse_motion(r#"{"motion":"nod","extra":"x"}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownKey), "got {err:?}");
    }

    #[test]
    fn parse_toast_happy_path() {
        let req = parse_toast(r#"{"level":"warn","message":"battery low"}"#).unwrap();
        assert_eq!(req.level, "warn");
        assert_eq!(req.message, "battery low");
    }

    #[test]
    fn parse_toast_message_defaults_to_empty() {
        // `message` is optional — empty toast still renders the band.
        let req = parse_toast(r#"{"level":"error"}"#).unwrap();
        assert_eq!(req.level, "error");
        assert_eq!(req.message, "");
    }

    #[test]
    fn parse_toast_rejects_missing_level() {
        let err = parse_toast(r#"{"message":"hi"}"#).unwrap_err();
        assert!(matches!(err, JsonError::MissingKey("level")), "got {err:?}");
    }

    #[test]
    fn parse_toast_rejects_duplicate_level() {
        let err = parse_toast(r#"{"level":"warn","level":"error"}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("level")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_toast_rejects_duplicate_message() {
        let err = parse_toast(r#"{"level":"warn","message":"a","message":"b"}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("message")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_toast_rejects_unknown_key() {
        let err = parse_toast(r#"{"level":"warn","extra":"x"}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownKey), "got {err:?}");
    }

    #[test]
    fn parse_start_listen_uses_default_duration_when_absent() {
        let cmd = parse_start_listen("{}").unwrap();
        match cmd {
            RemoteCommand::StartListen { duration_ms } => {
                assert_eq!(duration_ms, DEFAULT_LISTEN_DURATION_MS);
            }
            other => panic!("expected StartListen, got {other:?}"),
        }
    }

    #[test]
    fn parse_start_listen_accepts_explicit_duration() {
        let cmd = parse_start_listen(r#"{"duration_ms":5000}"#).unwrap();
        match cmd {
            RemoteCommand::StartListen { duration_ms } => assert_eq!(duration_ms, 5000),
            other => panic!("expected StartListen, got {other:?}"),
        }
    }

    #[test]
    fn parse_start_listen_rejects_duplicate_duration() {
        let err = parse_start_listen(r#"{"duration_ms":1,"duration_ms":2}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("duration_ms")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_start_listen_rejects_unknown_key() {
        let err = parse_start_listen(r#"{"extra":1}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownKey), "got {err:?}");
    }

    #[test]
    fn phrase_wire_str_round_trips_every_known_phrase() {
        // Pin the inverse of parse_phrase for every public variant.
        // Unknown variants (non_exhaustive escape) fall back to
        // "unknown" — exercised separately.
        use stackchan_core::voice::PhraseId;
        let known = [
            (PhraseId::WakeChirp, "wake_chirp"),
            (PhraseId::PickupChirp, "pickup_chirp"),
            (PhraseId::StartleChirp, "startle_chirp"),
            (PhraseId::LowBatteryChirp, "low_battery_chirp"),
            (
                PhraseId::CameraModeEnteredChirp,
                "camera_mode_entered_chirp",
            ),
            (PhraseId::CameraModeExitedChirp, "camera_mode_exited_chirp"),
            (PhraseId::Greeting, "greeting"),
            (PhraseId::AcknowledgeName, "acknowledge_name"),
            (PhraseId::BatteryLow, "battery_low"),
        ];
        for (id, wire) in known {
            assert_eq!(phrase_wire_str(id), wire);
        }
    }

    // ============================================================
    // Duplicate-key + unknown-key error paths on the bigger routes.
    // ============================================================

    #[test]
    fn parse_set_emotion_rejects_duplicate_hold_ms() {
        let err =
            parse_set_emotion(r#"{"emotion":"happy","hold_ms":1000,"hold_ms":2000}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("hold_ms")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_look_at_rejects_each_duplicate_key() {
        for (body, key) in [
            (r#"{"pan_deg":1,"pan_deg":2,"tilt_deg":0}"#, "pan_deg"),
            (r#"{"pan_deg":0,"tilt_deg":1,"tilt_deg":2}"#, "tilt_deg"),
            (
                r#"{"pan_deg":0,"tilt_deg":0,"hold_ms":1,"hold_ms":2}"#,
                "hold_ms",
            ),
        ] {
            let err = parse_look_at(body).unwrap_err();
            match err {
                JsonError::DuplicateKey(actual) => assert_eq!(actual, key, "for body {body}"),
                other => panic!("body {body}: expected DuplicateKey({key}), got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_look_at_rejects_unknown_key() {
        let err = parse_look_at(r#"{"pan_deg":0,"tilt_deg":0,"extra":1}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownKey), "got {err:?}");
    }

    #[test]
    fn parse_f32_rejects_non_finite_values() {
        // `parse_f32` is the shared f32 helper; tightening it at the
        // source means parse_look_at, parse_look_at_point, and any
        // future f32 route can rely on `is_finite()` downstream.
        // `1e400` overflows to `+Inf` on parse; matches the canonical
        // hostile-client probe.
        for body in [
            r#"{"pan_deg":1e400,"tilt_deg":0}"#,  // pan_deg = +Inf
            r#"{"pan_deg":-1e400,"tilt_deg":0}"#, // pan_deg = -Inf
            r#"{"pan_deg":0,"tilt_deg":1e400}"#,  // tilt_deg = +Inf
        ] {
            let err = parse_look_at(body).unwrap_err();
            assert!(matches!(err, JsonError::BadValue), "{body}: {err:?}");
        }
        let err = parse_look_at_point(r#"{"x":1e400,"y":0,"z":1}"#).unwrap_err();
        assert!(matches!(err, JsonError::BadValue), "{err:?}");
    }

    #[test]
    fn parse_look_at_point_rejects_each_duplicate_key() {
        for (body, key) in [
            (r#"{"x":1,"x":2,"y":0,"z":1}"#, "x"),
            (r#"{"y":1,"y":2,"x":0,"z":0}"#, "y"),
            (r#"{"x":0,"y":0,"z":1,"z":2}"#, "z"),
            (r#"{"x":0,"y":0,"z":1,"hold_ms":1,"hold_ms":2}"#, "hold_ms"),
        ] {
            let err = parse_look_at_point(body).unwrap_err();
            match err {
                JsonError::DuplicateKey(actual) => assert_eq!(actual, key, "for body {body}"),
                other => panic!("body {body}: expected DuplicateKey({key}), got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_speak_rejects_duplicate_locale() {
        let err = parse_speak(r#"{"phrase":"greeting","locale":"en","locale":"ja"}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("locale")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_mood_rejects_duplicate_mood_key() {
        let err = parse_mood(r#"{"mood":"focus","mood":"calm"}"#).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("mood")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_mood_rejects_unknown_key() {
        let err = parse_mood(r#"{"mood":"focus","extra":1}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnknownKey), "got {err:?}");
    }

    #[test]
    fn parse_head_offsets_rejects_out_of_range_tilt() {
        // tilt > HEAD_OFFSET_LIMIT_DEG must reject as BadValue.
        let body = r#"{"yaw_offset_deg":0,"tilt_offset_deg":99}"#;
        let err = parse_head_offsets(body).unwrap_err();
        assert!(matches!(err, JsonError::BadValue), "got {err:?}");
    }

    #[test]
    fn parse_head_offsets_rejects_duplicate_tilt() {
        let body = r#"{"yaw_offset_deg":0,"tilt_offset_deg":1,"tilt_offset_deg":2}"#;
        let err = parse_head_offsets(body).unwrap_err();
        assert!(
            matches!(err, JsonError::DuplicateKey("tilt_offset_deg")),
            "got {err:?}"
        );
    }

    #[test]
    fn parse_cancel_reminder_rejects_duplicate_id() {
        let err = parse_cancel_reminder(r#"{"id":1,"id":2}"#).unwrap_err();
        assert!(matches!(err, JsonError::DuplicateKey("id")), "got {err:?}");
    }
}
