//! Hand-rolled JSON-ish parser for the HTTP control plane's POST
//! bodies.
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
use stackchan_core::{Emotion, Pose, RemoteCommand};

/// Default hold window when the request body omits `hold_ms`.
pub const DEFAULT_HOLD_MS: u32 = 30_000;

/// Parser error surface — kept small; routes turn these into
/// `400 Bad Request` plain-text responses.
#[derive(Debug, defmt::Format)]
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

/// Parse a `POST /speak` body into a [`RemoteCommand::Speak`].
///
/// Required: `phrase` (string, lowercase `snake_case`). Optional:
/// `locale` (string, `"en"` / `"ja"`, defaults to `"en"`).
///
/// Priority is not on the wire — the firmware fills
/// [`Priority::Normal`] for every operator-driven request. Modifier-
/// internal call sites that need elevated priority go through
/// [`crate::audio::try_dispatch_utterance`] directly.
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
    /// `.`, `e`, `E`, `+`). The slice is parsed by the typed
    /// `parse_*` helpers via [`core::str::FromStr`].
    fn read_number(&mut self) -> Result<&'a str, JsonError> {
        self.skip_ws();
        let start = self.pos;
        while let Some(b) = self.peek() {
            let is_num = matches!(b, b'-' | b'+' | b'.' | b'0'..=b'9' | b'e' | b'E');
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
/// variant. Vocabulary is closed and lowercase.
fn parse_emotion(scanner: &mut Scanner<'_>) -> Result<Emotion, JsonError> {
    let raw = scanner.read_string()?;
    match raw {
        "neutral" => Ok(Emotion::Neutral),
        "happy" => Ok(Emotion::Happy),
        "sad" => Ok(Emotion::Sad),
        "sleepy" => Ok(Emotion::Sleepy),
        "surprised" => Ok(Emotion::Surprised),
        "angry" => Ok(Emotion::Angry),
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

/// Parse a contiguous number-shaped run as an `f32`.
fn parse_f32(scanner: &mut Scanner<'_>) -> Result<f32, JsonError> {
    scanner
        .read_number()?
        .parse::<f32>()
        .map_err(|_| JsonError::BadValue)
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
}
