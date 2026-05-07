//! Lip-sync data published alongside speech playback.
//!
//! [`LipSync`] always carries an `envelope` in `0.0..=1.0` (universally
//! producible — baked clips ship a sidecar curve, cloud APIs return
//! energy data, fallback is live RMS on outgoing samples). The
//! optional [`Viseme`] tag is supplied by backends that can emit
//! phoneme-level alignment; consumers prefer it when present and fall
//! back to envelope-only mouth shaping otherwise.
//!
//! Lives in core (not `stackchan-tts`) because the `Perception` layer
//! carries it as a per-frame field — `MouthFromAudio` switches between
//! mic-driven and TX-driven paths based on which is present.

/// Per-tick lip-sync hint. Always envelope-bearing; viseme is
/// best-effort.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct LipSync {
    /// Mouth-open amplitude in `0.0..=1.0`. Out-of-range values are
    /// clamped by the consumer; producers should clamp at source.
    pub envelope: f32,
    /// Phoneme tag, if the backend can supply one. `None` =
    /// "envelope only — pick a generic open shape."
    pub viseme: Option<Viseme>,
}

impl LipSync {
    /// Construct an envelope-only lip-sync hint.
    #[must_use]
    pub const fn envelope(amplitude: f32) -> Self {
        Self {
            envelope: amplitude,
            viseme: None,
        }
    }

    /// Construct a lip-sync hint with envelope + viseme.
    #[must_use]
    pub const fn with_viseme(envelope: f32, viseme: Viseme) -> Self {
        Self {
            envelope,
            viseme: Some(viseme),
        }
    }
}

/// Coarse phoneme classes for mouth-shape rendering.
///
/// Standard 8-class viseme set inspired by the JEFF/Disney mouth
/// chart; adequate for stylised avatar shapes without committing to a
/// full IPA-mapped inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Viseme {
    /// Lips closed (silence, /m/, /b/, /p/).
    Closed,
    /// Open low vowel (/a/, /ɑ/).
    Aa,
    /// Front vowel (/e/, /ɛ/).
    Ee,
    /// High front vowel (/i/, /ɪ/).
    Ii,
    /// Mid back vowel (/o/, /ɔ/).
    Oo,
    /// High back vowel (/u/, /ʊ/).
    Uu,
    /// Bilabial nasal-like (/m/, /n/).
    Mm,
    /// Labiodental fricative (/f/, /v/).
    Ff,
}

/// RMS silence floor for the viseme classifier.
///
/// Below this, the classifier returns [`Viseme::Closed`] regardless of
/// zero-crossing rate — a low-amplitude noise floor still has crossings
/// but isn't speech.
pub const VISEME_SILENCE_RMS: f32 = 0.03;

/// Low/mid ZCR boundary for the viseme classifier.
///
/// Frames below this rate land in [`Viseme::Aa`] (open vowels, nasals,
/// /o/-class vowels — all dominated by a single low formant).
pub const VISEME_LOW_ZCR_HZ: f32 = 800.0;

/// Mid/high ZCR boundary for the viseme classifier.
///
/// Frames between [`VISEME_LOW_ZCR_HZ`] and this threshold land in
/// [`Viseme::Ee`] (front vowels with brighter harmonics); above it,
/// [`Viseme::Ff`] (broadband fricative noise).
pub const VISEME_HIGH_ZCR_HZ: f32 = 2_000.0;

/// Classify a single audio frame into a coarse [`Viseme`] from its
/// RMS amplitude (`0.0..=1.0`, normalised against full-scale i16) and
/// zero-crossing rate (in Hz).
///
/// Implements the simple decision tree that the firmware's TX-side
/// lip-sync helper applies per DMA chunk:
///
/// | RMS                | ZCR (Hz)            | Viseme        |
/// |--------------------|---------------------|---------------|
/// | `< 0.03`           | (any)               | [`Closed`]    |
/// | `≥ 0.03`           | `< 800`             | [`Aa`]        |
/// | `≥ 0.03`           | `800..2_000`        | [`Ee`]        |
/// | `≥ 0.03`           | `≥ 2_000`           | [`Ff`]        |
///
/// [`Viseme::Oo`] / [`Viseme::Uu`] / [`Viseme::Mm`] / [`Viseme::Ii`]
/// aren't reachable from the RMS+ZCR signal alone — they need
/// formant analysis. The enum keeps those variants in reserve for
/// richer classifiers (e.g. phoneme timestamps from a `VoiceVox`
/// sidecar curve).
///
/// Non-finite inputs and negative RMS / ZCR values are treated as
/// silence to keep pathological audio from poisoning the avatar.
///
/// [`Closed`]: Viseme::Closed
/// [`Aa`]: Viseme::Aa
/// [`Ee`]: Viseme::Ee
/// [`Ff`]: Viseme::Ff
#[must_use]
pub fn classify_viseme(rms: f32, zcr_hz: f32) -> Viseme {
    // The `zcr_hz < 0.0` guard makes the function match its doc
    // contract — production callers (sign-flip counters) never produce
    // a negative rate, but a future caller deriving ZCR from a signed
    // correlation could, and a negative value would otherwise pass
    // through to `zcr_hz < VISEME_LOW_ZCR_HZ` and erroneously return
    // `Aa` for what's effectively garbage input.
    if !rms.is_finite() || !zcr_hz.is_finite() || rms < VISEME_SILENCE_RMS || zcr_hz < 0.0 {
        return Viseme::Closed;
    }
    if zcr_hz < VISEME_LOW_ZCR_HZ {
        Viseme::Aa
    } else if zcr_hz < VISEME_HIGH_ZCR_HZ {
        Viseme::Ee
    } else {
        Viseme::Ff
    }
}

impl Viseme {
    /// Multiplicative scale applied to the envelope when this viseme
    /// drives the mouth shape. `Closed` / `Mm` close the mouth fully;
    /// `Aa` / `Oo` keep the envelope as-is; mid-open and fricative
    /// shapes scale down so a sustained /s/ doesn't look identical to
    /// a sustained /a/.
    ///
    /// Multiplied into the existing envelope by `MouthFromAudio` —
    /// envelope still does the loud-vs-quiet work; viseme only adjusts
    /// shape.
    #[must_use]
    pub const fn mouth_scale(self) -> f32 {
        match self {
            Self::Closed | Self::Mm => 0.0,
            Self::Aa | Self::Oo => 1.0,
            Self::Ee | Self::Ii | Self::Uu => 0.5,
            Self::Ff => 0.3,
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;

    #[test]
    fn envelope_constructor_omits_viseme() {
        let l = LipSync::envelope(0.5);
        assert!((l.envelope - 0.5).abs() < f32::EPSILON);
        assert!(l.viseme.is_none());
    }

    #[test]
    fn with_viseme_sets_both() {
        let l = LipSync::with_viseme(0.8, Viseme::Aa);
        assert!((l.envelope - 0.8).abs() < f32::EPSILON);
        assert_eq!(l.viseme, Some(Viseme::Aa));
    }

    #[test]
    fn default_is_silent_envelope_only() {
        let l = LipSync::default();
        assert!(l.envelope.abs() < f32::EPSILON);
        assert!(l.viseme.is_none());
    }

    #[test]
    fn classifier_returns_closed_below_silence_rms() {
        // Anything quieter than the silence floor reads as Closed
        // regardless of ZCR — a low-amplitude noise floor still has
        // some zero crossings, but we don't want the mouth flapping.
        assert_eq!(classify_viseme(0.0, 0.0), Viseme::Closed);
        assert_eq!(classify_viseme(0.01, 100.0), Viseme::Closed);
        assert_eq!(
            classify_viseme(VISEME_SILENCE_RMS - 0.001, 5000.0),
            Viseme::Closed
        );
    }

    #[test]
    fn classifier_picks_aa_for_low_zcr_voiced_speech() {
        // Open vowels and nasals sit below ~800 Hz ZCR at speaking
        // fundamentals.
        assert_eq!(classify_viseme(0.5, 200.0), Viseme::Aa);
        assert_eq!(classify_viseme(0.5, 799.0), Viseme::Aa);
    }

    #[test]
    fn classifier_picks_ee_for_mid_zcr_voiced_speech() {
        // Front vowels with brighter harmonics.
        assert_eq!(classify_viseme(0.5, 800.0), Viseme::Ee);
        assert_eq!(classify_viseme(0.5, 1_500.0), Viseme::Ee);
        assert_eq!(classify_viseme(0.5, 1_999.0), Viseme::Ee);
    }

    #[test]
    fn classifier_picks_ff_for_high_zcr_fricatives() {
        // /s/ /f/ /sh/ — broadband noise reads as high ZCR.
        assert_eq!(classify_viseme(0.5, 2_000.0), Viseme::Ff);
        assert_eq!(classify_viseme(0.5, 5_000.0), Viseme::Ff);
    }

    #[test]
    fn classifier_treats_negative_zcr_as_silence() {
        // Production callers (sign-flip counters) can't produce a
        // negative rate, but a signed-correlation caller could; the
        // doc contract treats it as garbage and the guard must too.
        assert_eq!(classify_viseme(0.5, -1.0), Viseme::Closed);
        assert_eq!(classify_viseme(0.5, -10_000.0), Viseme::Closed);
    }

    #[test]
    fn classifier_treats_nonfinite_inputs_as_silence() {
        // NaN / ±Inf could come from a divide-by-zero in the
        // ZCR-to-Hz conversion if a chunk had zero samples; the
        // mouth shouldn't snap shut, but it shouldn't open either.
        assert_eq!(classify_viseme(f32::NAN, 1_000.0), Viseme::Closed);
        assert_eq!(classify_viseme(0.5, f32::NAN), Viseme::Closed);
        assert_eq!(classify_viseme(f32::INFINITY, 0.0), Viseme::Closed);
        assert_eq!(classify_viseme(0.5, f32::INFINITY), Viseme::Closed);
    }

    #[test]
    fn mouth_scale_closed_and_mm_silence_the_mouth() {
        assert!(Viseme::Closed.mouth_scale().abs() < f32::EPSILON);
        assert!(Viseme::Mm.mouth_scale().abs() < f32::EPSILON);
    }

    #[test]
    fn mouth_scale_aa_passes_envelope_through_unchanged() {
        // Aa is the "fully open" viseme — envelope drives the mouth
        // amplitude as-is.
        assert!((Viseme::Aa.mouth_scale() - 1.0).abs() < f32::EPSILON);
        assert!((Viseme::Oo.mouth_scale() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mouth_scale_fricative_is_smaller_than_voiced_open() {
        // /f/ / /s/ keep the mouth less open than /a/ — pin the
        // ordering so a future tweak can't quietly invert it.
        assert!(Viseme::Ff.mouth_scale() < Viseme::Aa.mouth_scale());
        assert!(Viseme::Ff.mouth_scale() < Viseme::Ee.mouth_scale());
    }
}
