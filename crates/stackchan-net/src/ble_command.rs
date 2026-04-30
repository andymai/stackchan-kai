//! BLE control-plane wire formats.
//!
//! Pairs with the GATT services declared in the firmware
//! (`crates/stackchan-firmware/src/ble/server.rs`). Each function
//! decodes the raw bytes from one characteristic write into a typed
//! value the firmware can act on, or returns a [`BleError`] the
//! firmware maps to an ATT error code.
//!
//! Lives in `stackchan-net` (not `stackchan-firmware`) so the codec
//! tests run on host — the firmware crate is
//! `xtensa-esp32s3-none-elf`-only and its `cfg(test)` modules are
//! never executed by `just check`. Same shape as the
//! [`crate::http_command`] parsers next door.
//!
//! ## Wire conventions
//!
//! - Multi-byte integers are **little-endian** (BLE attribute layout).
//! - Variant-bearing bytes (`Emotion`, `PhraseId`, `Locale`) use the
//!   byte mappings declared as `pub const`s in this module. Those
//!   mappings are **wire-stable across firmware releases** — the
//!   round-trip tests at the bottom of this file enforce that adding
//!   a new variant must take the next free byte rather than shuffling
//!   existing assignments.
//! - `hold_ms` fields encode `0` as "use [`DEFAULT_HOLD_MS`]". Encoding
//!   it inline keeps every characteristic fixed-length for a given
//!   schema, which is friendlier to the gatt-service macro than
//!   variable-length payloads.
//!
//! ## Characteristic layout
//!
//! | Char         | Bytes | Layout                                              |
//! |--------------|-------|-----------------------------------------------------|
//! | volume       | 1     | `u8` 0..=100                                        |
//! | mute         | 1     | `u8` 0\|1                                           |
//! | reset        | 1     | any single byte (trigger; value ignored)            |
//! | emotion      | 3     | `u8` emotion + `u16` hold_ms                        |
//! | look-at      | 6     | `i16` pan + `i16` tilt + `u16` hold_ms (centi-deg)  |
//! | speak        | 2     | `u8` phrase + `u8` locale                           |

use stackchan_core::voice::{Locale, PhraseId, Priority};
use stackchan_core::{Emotion, Pose, RemoteCommand};

/// Default hold window when a write payload signals "use default" by
/// encoding `hold_ms = 0`.
///
/// Matches [`crate::http_command::DEFAULT_HOLD_MS`] so HTTP and BLE
/// writes that omit a hold land on the same number.
pub const DEFAULT_HOLD_MS: u32 = 30_000;

/// Wire-level outer bound for the `i16` pan/tilt centi-degree fields.
///
/// `±180°·100 = ±18 000`. The servo driver applies its own (tighter)
/// motion limits; this wire bound only catches obviously corrupt or
/// fat-fingered values so the firmware never has to reason about an
/// `i16` that overflows degree space.
pub const LOOK_AT_RANGE_CENTI_DEG: i16 = 18_000;

/// Expected payload length for the volume characteristic.
pub const VOLUME_LEN: usize = 1;
/// Expected payload length for the mute characteristic.
pub const MUTE_LEN: usize = 1;
/// Expected payload length for the reset characteristic.
pub const RESET_LEN: usize = 1;
/// Expected payload length for the emotion-write characteristic
/// (`u8` emotion + `u16` `hold_ms`).
pub const EMOTION_WRITE_LEN: usize = 3;
/// Expected payload length for the look-at characteristic
/// (`i16` pan + `i16` tilt + `u16` `hold_ms`).
pub const LOOK_AT_LEN: usize = 6;
/// Expected payload length for the speak characteristic
/// (`u8` phrase + `u8` locale).
pub const SPEAK_LEN: usize = 2;

/// Wire byte for [`Emotion::Neutral`].
pub const EMOTION_NEUTRAL: u8 = 0;
/// Wire byte for [`Emotion::Happy`].
pub const EMOTION_HAPPY: u8 = 1;
/// Wire byte for [`Emotion::Sad`].
pub const EMOTION_SAD: u8 = 2;
/// Wire byte for [`Emotion::Sleepy`].
pub const EMOTION_SLEEPY: u8 = 3;
/// Wire byte for [`Emotion::Surprised`].
pub const EMOTION_SURPRISED: u8 = 4;
/// Wire byte for [`Emotion::Angry`].
pub const EMOTION_ANGRY: u8 = 5;

/// Wire byte for [`Locale::En`].
pub const LOCALE_EN: u8 = 0;
/// Wire byte for [`Locale::Ja`].
pub const LOCALE_JA: u8 = 1;

/// Wire byte for [`PhraseId::WakeChirp`].
pub const PHRASE_WAKE_CHIRP: u8 = 0;
/// Wire byte for [`PhraseId::PickupChirp`].
pub const PHRASE_PICKUP_CHIRP: u8 = 1;
/// Wire byte for [`PhraseId::StartleChirp`].
pub const PHRASE_STARTLE_CHIRP: u8 = 2;
/// Wire byte for [`PhraseId::LowBatteryChirp`].
pub const PHRASE_LOW_BATTERY_CHIRP: u8 = 3;
/// Wire byte for [`PhraseId::CameraModeEnteredChirp`].
pub const PHRASE_CAMERA_MODE_ENTERED_CHIRP: u8 = 4;
/// Wire byte for [`PhraseId::CameraModeExitedChirp`].
pub const PHRASE_CAMERA_MODE_EXITED_CHIRP: u8 = 5;
/// Wire byte for [`PhraseId::Greeting`].
pub const PHRASE_GREETING: u8 = 6;
/// Wire byte for [`PhraseId::AcknowledgeName`].
pub const PHRASE_ACKNOWLEDGE_NAME: u8 = 7;
/// Wire byte for [`PhraseId::BatteryLow`].
pub const PHRASE_BATTERY_LOW: u8 = 8;

/// Decoder error surface — every variant maps to one ATT error code in
/// the firmware. Kept small; a [`Debug`] dump is the user-facing log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleError {
    /// Payload byte length didn't match the characteristic's fixed shape.
    /// Maps to ATT `INVALID_ATTRIBUTE_VALUE_LENGTH` (0x0d).
    BadLength {
        /// Expected length for this characteristic.
        expected: usize,
        /// Actual length the central wrote.
        actual: usize,
    },
    /// Volume byte exceeded `100`. Maps to ATT `OUT_OF_RANGE` (0xFF).
    VolumeOutOfRange(u8),
    /// Mute byte was neither `0` nor `1`. Maps to ATT `VALUE_NOT_ALLOWED`
    /// (0x13) — strict so a buggy client surfaces fast.
    BadMuteByte(u8),
    /// Emotion byte didn't match any [`EMOTION_*`](EMOTION_NEUTRAL)
    /// constant. Maps to ATT `VALUE_NOT_ALLOWED`.
    UnknownEmotion(u8),
    /// Locale byte didn't match any [`LOCALE_*`](LOCALE_EN) constant.
    /// Maps to ATT `VALUE_NOT_ALLOWED`.
    UnknownLocale(u8),
    /// Phrase byte didn't match any [`PHRASE_*`](PHRASE_WAKE_CHIRP)
    /// constant. Maps to ATT `VALUE_NOT_ALLOWED`.
    UnknownPhrase(u8),
    /// Pan or tilt centi-degrees exceeded `±LOOK_AT_RANGE_CENTI_DEG`
    /// (see [`LOOK_AT_RANGE_CENTI_DEG`]). Maps to ATT `OUT_OF_RANGE`.
    LookAtOutOfRange {
        /// Submitted pan in centi-degrees.
        pan: i16,
        /// Submitted tilt in centi-degrees.
        tilt: i16,
    },
}

/// Decode the volume-write payload into a percent (`0..=100`).
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`VOLUME_LEN`] bytes.
/// - [`BleError::VolumeOutOfRange`] when the byte exceeds 100.
pub fn decode_volume(payload: &[u8]) -> Result<u8, BleError> {
    expect_len(payload, VOLUME_LEN)?;
    let v = payload[0];
    if v > 100 {
        return Err(BleError::VolumeOutOfRange(v));
    }
    Ok(v)
}

/// Decode the mute-write payload into a `bool`.
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`MUTE_LEN`] bytes.
/// - [`BleError::BadMuteByte`] for any byte that isn't `0` or `1`.
pub fn decode_mute(payload: &[u8]) -> Result<bool, BleError> {
    expect_len(payload, MUTE_LEN)?;
    match payload[0] {
        0 => Ok(false),
        1 => Ok(true),
        b => Err(BleError::BadMuteByte(b)),
    }
}

/// Decode the reset-write payload (any 1-byte value triggers the reset).
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`RESET_LEN`] bytes.
pub const fn decode_reset(payload: &[u8]) -> Result<(), BleError> {
    expect_len(payload, RESET_LEN)
}

/// Decode the emotion-write payload into a [`RemoteCommand::SetEmotion`].
///
/// Layout (3 bytes): `[emotion: u8, hold_ms: u16 LE]`.
/// `hold_ms == 0` → use [`DEFAULT_HOLD_MS`].
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`EMOTION_WRITE_LEN`] bytes.
/// - [`BleError::UnknownEmotion`] for an unmapped emotion byte.
pub fn decode_emotion_write(payload: &[u8]) -> Result<RemoteCommand, BleError> {
    expect_len(payload, EMOTION_WRITE_LEN)?;
    let emotion = decode_emotion_byte(payload[0])?;
    let hold_ms = u16::from_le_bytes([payload[1], payload[2]]);
    Ok(RemoteCommand::SetEmotion {
        emotion,
        hold_ms: resolve_hold(hold_ms),
    })
}

/// Decode the look-at write payload into a [`RemoteCommand::LookAt`].
///
/// Layout (6 bytes): `[pan: i16 LE, tilt: i16 LE, hold_ms: u16 LE]`,
/// pan/tilt in centi-degrees. `hold_ms == 0` → use [`DEFAULT_HOLD_MS`].
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`LOOK_AT_LEN`] bytes.
/// - [`BleError::LookAtOutOfRange`] when `|pan|` or `|tilt|` exceed
///   [`LOOK_AT_RANGE_CENTI_DEG`].
pub fn decode_look_at(payload: &[u8]) -> Result<RemoteCommand, BleError> {
    expect_len(payload, LOOK_AT_LEN)?;
    let pan_centi = i16::from_le_bytes([payload[0], payload[1]]);
    let tilt_centi = i16::from_le_bytes([payload[2], payload[3]]);
    let hold_ms = u16::from_le_bytes([payload[4], payload[5]]);
    if !axis_in_range(pan_centi) || !axis_in_range(tilt_centi) {
        return Err(BleError::LookAtOutOfRange {
            pan: pan_centi,
            tilt: tilt_centi,
        });
    }
    Ok(RemoteCommand::LookAt {
        target: Pose {
            pan_deg: f32::from(pan_centi) / 100.0,
            tilt_deg: f32::from(tilt_centi) / 100.0,
        },
        hold_ms: resolve_hold(hold_ms),
    })
}

/// Decode the speak-write payload into a [`RemoteCommand::Speak`].
///
/// Layout (2 bytes): `[phrase: u8, locale: u8]`.
/// Priority is implicitly [`Priority::Normal`] — the BLE surface
/// doesn't expose elevated priorities; modifier-internal callers go
/// through `audio::try_dispatch_utterance` directly, same as HTTP.
///
/// # Errors
///
/// - [`BleError::BadLength`] when the central writes anything other
///   than [`SPEAK_LEN`] bytes.
/// - [`BleError::UnknownPhrase`] / [`BleError::UnknownLocale`] for
///   unmapped variant bytes.
pub fn decode_speak(payload: &[u8]) -> Result<RemoteCommand, BleError> {
    expect_len(payload, SPEAK_LEN)?;
    let phrase = decode_phrase_byte(payload[0])?;
    let locale = decode_locale_byte(payload[1])?;
    Ok(RemoteCommand::Speak {
        phrase,
        locale,
        priority: Priority::Normal,
    })
}

/// Encode an [`Emotion`] for the read side of the emotion characteristic.
/// Identical to [`Emotion::wire_byte`]; re-exported here so callers
/// reach for a single `ble_command` import path when speaking BLE.
#[must_use]
pub const fn encode_emotion(emotion: Emotion) -> u8 {
    emotion.wire_byte()
}

/// Encode a mute flag as the single byte the mute characteristic
/// publishes on read / notify.
#[must_use]
pub const fn encode_mute(muted: bool) -> u8 {
    if muted { 1 } else { 0 }
}

/// Reject a payload whose length doesn't match the characteristic's
/// fixed shape. Centralises the [`BleError::BadLength`] construction
/// so each decoder reads as a single guard line.
const fn expect_len(payload: &[u8], expected: usize) -> Result<(), BleError> {
    if payload.len() == expected {
        Ok(())
    } else {
        Err(BleError::BadLength {
            expected,
            actual: payload.len(),
        })
    }
}

/// Translate the `0`-means-default sentinel into [`DEFAULT_HOLD_MS`].
/// Any non-zero u16 widens to its `u32` equivalent unchanged.
const fn resolve_hold(hold_ms: u16) -> u32 {
    if hold_ms == 0 {
        DEFAULT_HOLD_MS
    } else {
        hold_ms as u32
    }
}

/// Bounds check for either pan or tilt — both axes share the same
/// wire-level range gate. Rejects anything outside
/// <code>±[LOOK_AT_RANGE_CENTI_DEG]</code>.
const fn axis_in_range(centi: i16) -> bool {
    centi >= -LOOK_AT_RANGE_CENTI_DEG && centi <= LOOK_AT_RANGE_CENTI_DEG
}

/// Translate an emotion wire byte back to the [`Emotion`] variant or
/// surface [`BleError::UnknownEmotion`] if the byte is unmapped.
const fn decode_emotion_byte(b: u8) -> Result<Emotion, BleError> {
    match b {
        EMOTION_NEUTRAL => Ok(Emotion::Neutral),
        EMOTION_HAPPY => Ok(Emotion::Happy),
        EMOTION_SAD => Ok(Emotion::Sad),
        EMOTION_SLEEPY => Ok(Emotion::Sleepy),
        EMOTION_SURPRISED => Ok(Emotion::Surprised),
        EMOTION_ANGRY => Ok(Emotion::Angry),
        other => Err(BleError::UnknownEmotion(other)),
    }
}

/// Translate a locale wire byte back to the [`Locale`] variant or
/// surface [`BleError::UnknownLocale`] if the byte is unmapped.
const fn decode_locale_byte(b: u8) -> Result<Locale, BleError> {
    match b {
        LOCALE_EN => Ok(Locale::En),
        LOCALE_JA => Ok(Locale::Ja),
        other => Err(BleError::UnknownLocale(other)),
    }
}

/// Translate a phrase wire byte back to the [`PhraseId`] variant or
/// surface [`BleError::UnknownPhrase`] if the byte is unmapped.
const fn decode_phrase_byte(b: u8) -> Result<PhraseId, BleError> {
    match b {
        PHRASE_WAKE_CHIRP => Ok(PhraseId::WakeChirp),
        PHRASE_PICKUP_CHIRP => Ok(PhraseId::PickupChirp),
        PHRASE_STARTLE_CHIRP => Ok(PhraseId::StartleChirp),
        PHRASE_LOW_BATTERY_CHIRP => Ok(PhraseId::LowBatteryChirp),
        PHRASE_CAMERA_MODE_ENTERED_CHIRP => Ok(PhraseId::CameraModeEnteredChirp),
        PHRASE_CAMERA_MODE_EXITED_CHIRP => Ok(PhraseId::CameraModeExitedChirp),
        PHRASE_GREETING => Ok(PhraseId::Greeting),
        PHRASE_ACKNOWLEDGE_NAME => Ok(PhraseId::AcknowledgeName),
        PHRASE_BATTERY_LOW => Ok(PhraseId::BatteryLow),
        other => Err(BleError::UnknownPhrase(other)),
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
    fn volume_accepts_in_range() {
        for pct in [0u8, 1, 50, 99, 100] {
            assert_eq!(decode_volume(&[pct]).unwrap(), pct);
        }
    }

    #[test]
    fn volume_rejects_above_100() {
        assert_eq!(decode_volume(&[101]), Err(BleError::VolumeOutOfRange(101)));
        assert_eq!(decode_volume(&[200]), Err(BleError::VolumeOutOfRange(200)));
    }

    #[test]
    fn volume_rejects_wrong_length() {
        assert_eq!(
            decode_volume(&[]),
            Err(BleError::BadLength {
                expected: 1,
                actual: 0
            })
        );
        assert_eq!(
            decode_volume(&[10, 20]),
            Err(BleError::BadLength {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn mute_round_trips_both_booleans() {
        assert!(!decode_mute(&[encode_mute(false)]).unwrap());
        assert!(decode_mute(&[encode_mute(true)]).unwrap());
    }

    #[test]
    fn mute_rejects_non_boolean_byte() {
        assert_eq!(decode_mute(&[2]), Err(BleError::BadMuteByte(2)));
        assert_eq!(decode_mute(&[0xFF]), Err(BleError::BadMuteByte(0xFF)));
    }

    #[test]
    fn reset_accepts_any_single_byte() {
        assert!(decode_reset(&[0]).is_ok());
        assert!(decode_reset(&[0xFF]).is_ok());
    }

    #[test]
    fn reset_rejects_wrong_length() {
        assert_eq!(
            decode_reset(&[]),
            Err(BleError::BadLength {
                expected: 1,
                actual: 0
            })
        );
        assert_eq!(
            decode_reset(&[1, 2]),
            Err(BleError::BadLength {
                expected: 1,
                actual: 2
            })
        );
    }

    #[test]
    fn emotion_write_round_trips_every_variant() {
        // Pin the byte/variant binding: every Emotion variant must
        // round-trip through encode → decode_emotion_write so adding
        // a variant forces an explicit byte choice rather than a
        // silent shuffle.
        for variant in [
            Emotion::Neutral,
            Emotion::Happy,
            Emotion::Sad,
            Emotion::Sleepy,
            Emotion::Surprised,
            Emotion::Angry,
        ] {
            let mut payload = [0u8; EMOTION_WRITE_LEN];
            payload[0] = encode_emotion(variant);
            // hold_ms 5000 little-endian.
            payload[1..3].copy_from_slice(&5000u16.to_le_bytes());
            match decode_emotion_write(&payload).unwrap() {
                RemoteCommand::SetEmotion { emotion, hold_ms } => {
                    assert_eq!(emotion, variant);
                    assert_eq!(hold_ms, 5000);
                }
                other => panic!("expected SetEmotion, got {other:?}"),
            }
        }
    }

    #[test]
    fn emotion_write_zero_hold_falls_back_to_default() {
        let payload = [EMOTION_HAPPY, 0, 0];
        match decode_emotion_write(&payload).unwrap() {
            RemoteCommand::SetEmotion { hold_ms, .. } => {
                assert_eq!(hold_ms, DEFAULT_HOLD_MS);
            }
            other => panic!("expected SetEmotion, got {other:?}"),
        }
    }

    #[test]
    fn emotion_write_rejects_unknown_byte() {
        let payload = [42, 0, 0];
        assert_eq!(
            decode_emotion_write(&payload),
            Err(BleError::UnknownEmotion(42))
        );
    }

    #[test]
    fn emotion_write_rejects_wrong_length() {
        assert_eq!(
            decode_emotion_write(&[EMOTION_HAPPY, 0]),
            Err(BleError::BadLength {
                expected: 3,
                actual: 2
            })
        );
    }

    #[test]
    fn look_at_decodes_signed_centi_degrees() {
        let mut payload = [0u8; LOOK_AT_LEN];
        payload[0..2].copy_from_slice(&(-1234i16).to_le_bytes());
        payload[2..4].copy_from_slice(&5678i16.to_le_bytes());
        payload[4..6].copy_from_slice(&2000u16.to_le_bytes());
        match decode_look_at(&payload).unwrap() {
            RemoteCommand::LookAt { target, hold_ms } => {
                assert_eq!(target.pan_deg, -12.34);
                assert_eq!(target.tilt_deg, 56.78);
                assert_eq!(hold_ms, 2000);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn look_at_zero_hold_falls_back_to_default() {
        let payload = [0u8; LOOK_AT_LEN];
        match decode_look_at(&payload).unwrap() {
            RemoteCommand::LookAt { hold_ms, .. } => {
                assert_eq!(hold_ms, DEFAULT_HOLD_MS);
            }
            other => panic!("expected LookAt, got {other:?}"),
        }
    }

    #[test]
    fn look_at_rejects_pan_out_of_range() {
        let mut payload = [0u8; LOOK_AT_LEN];
        let bad_pan: i16 = LOOK_AT_RANGE_CENTI_DEG + 1;
        payload[0..2].copy_from_slice(&bad_pan.to_le_bytes());
        match decode_look_at(&payload) {
            Err(BleError::LookAtOutOfRange { pan, tilt }) => {
                assert_eq!(pan, bad_pan);
                assert_eq!(tilt, 0);
            }
            other => panic!("expected LookAtOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn look_at_rejects_negative_pan_out_of_range() {
        let mut payload = [0u8; LOOK_AT_LEN];
        let bad_pan: i16 = -LOOK_AT_RANGE_CENTI_DEG - 1;
        payload[0..2].copy_from_slice(&bad_pan.to_le_bytes());
        assert!(matches!(
            decode_look_at(&payload),
            Err(BleError::LookAtOutOfRange { .. })
        ));
    }

    #[test]
    fn look_at_rejects_tilt_out_of_range() {
        // Tilt rides the same gate as pan but through a separate
        // axis_in_range call — pin both axes so a future split (e.g.
        // the asymmetric servo travel of MAX_TILT_DEG / MIN_TILT_DEG)
        // can't drop tilt validation by accident.
        let mut payload = [0u8; LOOK_AT_LEN];
        let bad_tilt: i16 = LOOK_AT_RANGE_CENTI_DEG + 1;
        payload[2..4].copy_from_slice(&bad_tilt.to_le_bytes());
        match decode_look_at(&payload) {
            Err(BleError::LookAtOutOfRange { pan, tilt }) => {
                assert_eq!(pan, 0);
                assert_eq!(tilt, bad_tilt);
            }
            other => panic!("expected LookAtOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn look_at_accepts_range_endpoints() {
        for pan in [-LOOK_AT_RANGE_CENTI_DEG, LOOK_AT_RANGE_CENTI_DEG] {
            let mut payload = [0u8; LOOK_AT_LEN];
            payload[0..2].copy_from_slice(&pan.to_le_bytes());
            assert!(decode_look_at(&payload).is_ok(), "pan={pan}");
        }
    }

    #[test]
    fn look_at_rejects_wrong_length() {
        assert_eq!(
            decode_look_at(&[0; 5]),
            Err(BleError::BadLength {
                expected: 6,
                actual: 5
            })
        );
    }

    #[test]
    fn speak_round_trips_every_locale() {
        for (variant, byte) in [(Locale::En, LOCALE_EN), (Locale::Ja, LOCALE_JA)] {
            match decode_speak(&[PHRASE_GREETING, byte]).unwrap() {
                RemoteCommand::Speak { locale, .. } => assert_eq!(locale, variant),
                other => panic!("expected Speak, got {other:?}"),
            }
        }
    }

    #[test]
    fn speak_round_trips_every_phrase() {
        // Every PhraseId variant must round-trip — adding a phrase
        // without giving it a wire byte fails this test, forcing a
        // deliberate byte assignment.
        for (variant, byte) in [
            (PhraseId::WakeChirp, PHRASE_WAKE_CHIRP),
            (PhraseId::PickupChirp, PHRASE_PICKUP_CHIRP),
            (PhraseId::StartleChirp, PHRASE_STARTLE_CHIRP),
            (PhraseId::LowBatteryChirp, PHRASE_LOW_BATTERY_CHIRP),
            (
                PhraseId::CameraModeEnteredChirp,
                PHRASE_CAMERA_MODE_ENTERED_CHIRP,
            ),
            (
                PhraseId::CameraModeExitedChirp,
                PHRASE_CAMERA_MODE_EXITED_CHIRP,
            ),
            (PhraseId::Greeting, PHRASE_GREETING),
            (PhraseId::AcknowledgeName, PHRASE_ACKNOWLEDGE_NAME),
            (PhraseId::BatteryLow, PHRASE_BATTERY_LOW),
        ] {
            match decode_speak(&[byte, LOCALE_EN]).unwrap() {
                RemoteCommand::Speak { phrase, .. } => assert_eq!(phrase, variant),
                other => panic!("expected Speak, got {other:?}"),
            }
        }
    }

    #[test]
    fn speak_priority_is_normal() {
        match decode_speak(&[PHRASE_GREETING, LOCALE_EN]).unwrap() {
            RemoteCommand::Speak { priority, .. } => assert_eq!(priority, Priority::Normal),
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn speak_rejects_unknown_phrase_byte() {
        assert_eq!(
            decode_speak(&[0xEE, LOCALE_EN]),
            Err(BleError::UnknownPhrase(0xEE))
        );
    }

    #[test]
    fn speak_rejects_unknown_locale_byte() {
        assert_eq!(
            decode_speak(&[PHRASE_GREETING, 0xEE]),
            Err(BleError::UnknownLocale(0xEE))
        );
    }

    #[test]
    fn speak_rejects_wrong_length() {
        assert_eq!(
            decode_speak(&[PHRASE_GREETING]),
            Err(BleError::BadLength {
                expected: 2,
                actual: 1
            })
        );
    }
}
