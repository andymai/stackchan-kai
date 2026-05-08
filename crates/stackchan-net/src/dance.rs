//! JSON parser for the dance HTTP route.
//!
//! Schema (`DanceScript` / `Keyframe`) lives in
//! [`stackchan_core::dance`] so the player modifier (also in core)
//! can consume it without inverting the dependency direction. This
//! module wraps the schema with JSON parsing, mapping the typed
//! [`stackchan_core::dance::DanceError`] into [`JsonError`] so the
//! HTTP route returns a uniform error surface.
//!
//! ## Wire format
//!
//! Live HTTP keyframe streams use JSON (matches the existing
//! `bare_json` infrastructure on the firmware target). Future
//! RON-on-SD canned scripts can land alongside this module without
//! changing the schema.
//!
//! ```text
//! POST /dance
//! {
//!   "keyframes": [
//!     {"at_ms": 0,    "emotion": "happy",        "r": 255, "g": 200, "b": 0},
//!     {"at_ms": 0,    "pan_deg": -20.0, "tilt_deg": 10.0},
//!     {"at_ms": 500,  "pan_deg":  20.0},
//!     {"at_ms": 1000, "pan_deg": -20.0, "decorator": "heart"},
//!     {"at_ms": 1500, "pan_deg":   0.0, "tilt_deg": 0.0}
//!   ]
//! }
//! ```
//!
//! ## Sampling
//!
//! The player ([`stackchan_core::modifiers::DancePlayer`]) picks the
//! most-recent keyframe at-or-before `now` per channel — no
//! interpolation between keyframes for any channel. For motion, the
//! head driver's existing slew limit smooths the step changes (the
//! `SCServo`'s internal 20 ms move-time interpolator does it
//! implicitly). For avatar and RGB, step changes are the intended
//! visual: an emotion swap reads as a deliberate beat.

use alloc::vec::Vec;

use stackchan_core::Decorator;
use stackchan_core::dance::{DanceError, DanceScript, Keyframe, MAX_KEYFRAMES, validate};

use crate::http_command::{JsonError, Scanner, parse_emotion, parse_f32, parse_u32};

/// Translate a domain-level [`DanceError`] from the schema validator
/// into the parser's [`JsonError`] surface.
///
/// Every current variant collapses to [`JsonError::BadValue`] — the
/// HTTP route doesn't differentiate authoring failures at the wire
/// level. The `_` arm covers `DanceError`'s `#[non_exhaustive]`
/// future variants without code churn.
const fn dance_error_to_json(_err: DanceError) -> JsonError {
    JsonError::BadValue
}

/// Parse a `POST /dance` JSON body into a [`DanceScript`].
///
/// Body shape: a top-level object with a single `keyframes` array.
/// Each array element is a [`Keyframe`] object with `at_ms`
/// required and any subset of channel fields optional.
///
/// # Errors
///
/// Returns [`JsonError`] for any structural malformation, unknown
/// keys, missing `at_ms`, partial RGB triples, or other validation
/// failures (see [`validate`]).
pub fn parse_dance(body: &str) -> Result<DanceScript, JsonError> {
    let mut scanner = Scanner::new(body);
    // Top-level object with one required key: "keyframes".
    scanner.expect(b'{')?;
    let mut keyframes: Option<Vec<Keyframe>> = None;
    let mut first = true;
    loop {
        scanner.skip_ws();
        if scanner.peek() == Some(b'}') {
            let _ = scanner.bump();
            break;
        }
        if !first {
            scanner.expect(b',')?;
        }
        first = false;
        let key = scanner.read_string()?;
        scanner.expect(b':')?;
        match key {
            "keyframes" => {
                if keyframes.is_some() {
                    return Err(JsonError::DuplicateKey("keyframes"));
                }
                keyframes = Some(parse_keyframe_array(&mut scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
    }
    scanner.skip_ws();
    // Anything after the closing brace is a structural error.
    if scanner.peek().is_some() {
        return Err(JsonError::Unterminated);
    }
    let script = DanceScript {
        keyframes: keyframes.ok_or(JsonError::MissingKey("keyframes"))?,
    };
    validate(&script).map_err(dance_error_to_json)?;
    Ok(script)
}

/// Walk `[ {keyframe}, {keyframe}, ... ]` from the scanner's current
/// position. The opening `[` is consumed by this function; the closing
/// `]` is consumed before return.
fn parse_keyframe_array(scanner: &mut Scanner<'_>) -> Result<Vec<Keyframe>, JsonError> {
    scanner.expect(b'[')?;
    let mut out: Vec<Keyframe> = Vec::new();
    scanner.skip_ws();
    if scanner.peek() == Some(b']') {
        let _ = scanner.bump();
        return Ok(out);
    }
    loop {
        if out.len() >= MAX_KEYFRAMES {
            return Err(JsonError::BadValue);
        }
        out.push(parse_keyframe(scanner)?);
        scanner.skip_ws();
        match scanner.bump() {
            Some(b',') => {}
            Some(b']') => break,
            _ => return Err(JsonError::Unterminated),
        }
    }
    Ok(out)
}

/// Walk a single `{ at_ms: ..., ... }` object from the scanner's
/// current position into a [`Keyframe`].
fn parse_keyframe(scanner: &mut Scanner<'_>) -> Result<Keyframe, JsonError> {
    scanner.expect(b'{')?;
    let mut frame = Keyframe::default();
    let mut at_ms_seen = false;
    let mut first = true;
    loop {
        scanner.skip_ws();
        if scanner.peek() == Some(b'}') {
            let _ = scanner.bump();
            break;
        }
        if !first {
            scanner.expect(b',')?;
        }
        first = false;
        let key = scanner.read_string()?;
        scanner.expect(b':')?;
        match key {
            "at_ms" => {
                if at_ms_seen {
                    return Err(JsonError::DuplicateKey("at_ms"));
                }
                frame.at_ms = parse_u32(scanner)?;
                at_ms_seen = true;
            }
            "pan_deg" => {
                if frame.pan_deg.is_some() {
                    return Err(JsonError::DuplicateKey("pan_deg"));
                }
                frame.pan_deg = Some(parse_f32(scanner)?);
            }
            "tilt_deg" => {
                if frame.tilt_deg.is_some() {
                    return Err(JsonError::DuplicateKey("tilt_deg"));
                }
                frame.tilt_deg = Some(parse_f32(scanner)?);
            }
            "emotion" => {
                if frame.emotion.is_some() {
                    return Err(JsonError::DuplicateKey("emotion"));
                }
                frame.emotion = Some(parse_emotion(scanner)?);
            }
            "decorator" => {
                if frame.decorator.is_some() {
                    return Err(JsonError::DuplicateKey("decorator"));
                }
                frame.decorator = Some(parse_decorator(scanner)?);
            }
            "r" => {
                if frame.r.is_some() {
                    return Err(JsonError::DuplicateKey("r"));
                }
                frame.r = Some(parse_u8(scanner)?);
            }
            "g" => {
                if frame.g.is_some() {
                    return Err(JsonError::DuplicateKey("g"));
                }
                frame.g = Some(parse_u8(scanner)?);
            }
            "b" => {
                if frame.b.is_some() {
                    return Err(JsonError::DuplicateKey("b"));
                }
                frame.b = Some(parse_u8(scanner)?);
            }
            _ => return Err(JsonError::UnknownKey),
        }
    }
    if !at_ms_seen {
        return Err(JsonError::MissingKey("at_ms"));
    }
    Ok(frame)
}

/// Parse a `"<decorator>"` string into a [`Decorator`] variant.
/// Vocabulary mirrors [`Decorator::wire_str`] — exhaustive match
/// rejects anything outside the known set.
fn parse_decorator(scanner: &mut Scanner<'_>) -> Result<Decorator, JsonError> {
    let raw = scanner.read_string()?;
    Decorator::ALL
        .iter()
        .copied()
        .find(|d| d.wire_str() == raw)
        .ok_or(JsonError::BadValue)
}

/// Parse a JSON integer in `0..=255` for an RGB component.
fn parse_u8(scanner: &mut Scanner<'_>) -> Result<u8, JsonError> {
    let v = parse_u32(scanner)?;
    u8::try_from(v).map_err(|_| JsonError::BadValue)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test-only: unwrap is the standard pattern for asserting Some-returning helpers"
)]
mod tests {
    use super::*;
    use stackchan_core::{Decorator, Emotion};

    #[test]
    fn parse_minimal_script() {
        let body = r#"{"keyframes":[{"at_ms":0,"pan_deg":-20.0}]}"#;
        let script = parse_dance(body).unwrap();
        assert_eq!(script.keyframes.len(), 1);
        assert_eq!(script.keyframes[0].at_ms, 0);
        assert_eq!(script.keyframes[0].pan_deg, Some(-20.0));
    }

    #[test]
    fn parse_full_keyframe_with_all_channels() {
        let body = r#"{"keyframes":[
            {"at_ms":0,"pan_deg":-20.0,"tilt_deg":10.0,"emotion":"happy","decorator":"heart","r":255,"g":200,"b":0}
        ]}"#;
        let script = parse_dance(body).unwrap();
        let frame = &script.keyframes[0];
        assert_eq!(frame.pan_deg, Some(-20.0));
        assert_eq!(frame.tilt_deg, Some(10.0));
        assert_eq!(frame.emotion, Some(Emotion::Happy));
        assert_eq!(frame.decorator, Some(Decorator::Heart));
        assert_eq!(frame.rgb(), Some((255, 200, 0)));
    }

    #[test]
    fn parse_multi_keyframe_script() {
        let body = r#"{"keyframes":[
            {"at_ms":0,"emotion":"happy","r":255,"g":200,"b":0},
            {"at_ms":500,"pan_deg":20.0},
            {"at_ms":1000,"pan_deg":-20.0,"decorator":"heart"},
            {"at_ms":1500,"pan_deg":0.0,"tilt_deg":0.0}
        ]}"#;
        let script = parse_dance(body).unwrap();
        assert_eq!(script.keyframes.len(), 4);
        assert_eq!(script.keyframes[0].emotion, Some(Emotion::Happy));
        assert_eq!(script.keyframes[1].pan_deg, Some(20.0));
        assert_eq!(script.keyframes[2].decorator, Some(Decorator::Heart));
        assert_eq!(script.keyframes[3].tilt_deg, Some(0.0));
    }

    #[test]
    fn parse_rejects_missing_at_ms() {
        let body = r#"{"keyframes":[{"pan_deg":0.0}]}"#;
        assert!(matches!(
            parse_dance(body),
            Err(JsonError::MissingKey("at_ms"))
        ));
    }

    #[test]
    fn parse_rejects_missing_keyframes() {
        let body = "{}";
        assert!(matches!(
            parse_dance(body),
            Err(JsonError::MissingKey("keyframes"))
        ));
    }

    #[test]
    fn parse_rejects_unknown_top_level_key() {
        let body = r#"{"unknown":1,"keyframes":[]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::UnknownKey)));
    }

    #[test]
    fn parse_rejects_unknown_keyframe_key() {
        let body = r#"{"keyframes":[{"at_ms":0,"speed":1.0}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::UnknownKey)));
    }

    #[test]
    fn parse_rejects_duplicate_keyframe_field() {
        let body = r#"{"keyframes":[{"at_ms":0,"at_ms":1}]}"#;
        assert!(matches!(
            parse_dance(body),
            Err(JsonError::DuplicateKey("at_ms"))
        ));
    }

    #[test]
    fn parse_rejects_partial_rgb() {
        // Missing `b` should hit the validator's partial-RGB check.
        let body = r#"{"keyframes":[{"at_ms":0,"r":10,"g":20}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn parse_rejects_out_of_order_keyframes() {
        let body = r#"{"keyframes":[{"at_ms":100},{"at_ms":50}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn parse_rejects_empty_keyframes() {
        let body = r#"{"keyframes":[]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn parse_rejects_rgb_out_of_range() {
        let body = r#"{"keyframes":[{"at_ms":0,"r":300,"g":0,"b":0}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn parse_tolerates_whitespace_and_newlines() {
        let body = r#"
            {
              "keyframes": [
                { "at_ms":   0, "pan_deg":  -20.0 },
                { "at_ms": 500, "pan_deg":   20.0 }
              ]
            }
        "#;
        let script = parse_dance(body).unwrap();
        assert_eq!(script.keyframes.len(), 2);
    }

    #[test]
    fn parse_rejects_unknown_decorator() {
        let body = r#"{"keyframes":[{"at_ms":0,"decorator":"nope"}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::BadValue)));
    }

    #[test]
    fn parse_rejects_unknown_emotion() {
        let body = r#"{"keyframes":[{"at_ms":0,"emotion":"furious"}]}"#;
        assert!(matches!(parse_dance(body), Err(JsonError::UnknownEmotion)));
    }
}
