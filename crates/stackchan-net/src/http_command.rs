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
use stackchan_core::{Emotion, Mood, Pose, RemoteCommand};

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
    /// `audio.volume_pct` value outside the documented `0..=100`
    /// range. Carries the offending value so the firmware's `400`
    /// response body is self-describing.
    VolumeOutOfRange(u16),
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

/// Single-pass byte cursor over the body. Each parse helper advances
/// past the value it consumes (without consuming the trailing comma
/// or `}` — those belong to [`visit_object`]).
struct Scanner<'a> {
    /// The body's raw bytes.
    bytes: &'a [u8],
    /// Read position into [`Scanner::bytes`].
    pos: usize,
}

impl<'a> Scanner<'a> {
    /// Construct a scanner positioned at the start of `input`.
    const fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    /// Advance past any ASCII whitespace at the current position.
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// Peek the byte at the current position without advancing.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Read the byte at the current position and advance one byte.
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Skip whitespace and require the next byte to be `byte`.
    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
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
    fn read_string(&mut self) -> Result<&'a str, JsonError> {
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
    fn read_number(&mut self) -> Result<&'a str, JsonError> {
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
fn parse_emotion(scanner: &mut Scanner<'_>) -> Result<Emotion, JsonError> {
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
fn parse_u32(scanner: &mut Scanner<'_>) -> Result<u32, JsonError> {
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

/// Parse a contiguous number-shaped run as an `f32`.
fn parse_f32(scanner: &mut Scanner<'_>) -> Result<f32, JsonError> {
    scanner
        .read_number()?
        .parse::<f32>()
        .map_err(|_| JsonError::BadValue)
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
}
