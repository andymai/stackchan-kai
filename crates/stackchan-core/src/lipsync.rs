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
}
