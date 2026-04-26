//! [`BakedBackend`] — renders [`SpeechContent::Phrase`] catalog
//! entries from compile-time-baked tables.
//!
//! Two flavours share the backend:
//!
//! - **Non-verbal SFX** ([`PhraseId::WakeChirp`] et al.) render via
//!   sine-cycle tables looped N times — same approach the firmware
//!   used for chirps before they were subsumed under [`PhraseId`].
//!   Tables live in this module so the backend is self-contained.
//! - **Verbal phrases** ([`PhraseId::Greeting`] et al.) render from
//!   raw 16 kHz/16-bit mono PCM committed under
//!   `crates/stackchan-tts/assets/<locale>/<phrase>.pcm` and embedded
//!   via `include_bytes!`. Until the asset pipeline lands (Stage 6 of
//!   the TTS rollout), these variants return [`RenderError::AssetMissing`].
//!
//! [`SpeechContent::Phrase`]: stackchan_core::voice::SpeechContent::Phrase
//! [`PhraseId`]: stackchan_core::voice::PhraseId
//! [`PhraseId::WakeChirp`]: stackchan_core::voice::PhraseId::WakeChirp
//! [`PhraseId::Greeting`]: stackchan_core::voice::PhraseId::Greeting

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use stackchan_core::voice::{Locale, PhraseId, SpeechContent, Utterance};

use crate::backend::{RenderError, SpeechBackend};
use crate::source::AudioSource;

// =====================================================================
// Sine-cycle tables — 16 kHz mono i16, amplitude 8192 (≈ -12 dBFS).
// Pre-baked at compile time so the firmware crate stays libm-free.
// =====================================================================

/// One cycle of 1 kHz at 16 kHz sample rate (16 samples).
const SINE_1KHZ: &[i16] = &[
    0, 3135, 5793, 7568, 8192, 7568, 5793, 3135, 0, -3135, -5793, -7568, -8192, -7568, -5793, -3135,
];
/// One cycle of 2 kHz at 16 kHz sample rate (8 samples).
const SINE_2KHZ: &[i16] = &[0, 5793, 8192, 5793, 0, -5793, -8192, -5793];
/// One cycle of 4 kHz at 16 kHz sample rate (4 samples). Highest pitch
/// playable cleanly without breaching Nyquist.
const SINE_4KHZ: &[i16] = &[0, 8192, 0, -8192];
/// 8-sample silence cycle. Used for inter-pulse gaps in
/// [`PhraseId::LowBatteryChirp`].
const SILENCE_8: &[i16] = &[0; 8];

// =====================================================================
// AudioSource implementations
// =====================================================================

/// PCM source that loops `samples` `cycles_remaining` times, then
/// exhausts. Mirrors the firmware's prior `ClipPlayback` shape but
/// implements [`AudioSource`] with a bulk [`AudioSource::fill`].
#[derive(Debug)]
pub struct SineTableSource {
    /// One cycle of the wave. Read repeatedly; never mutated.
    samples: &'static [i16],
    /// Index into `samples` of the next sample to emit.
    cursor: usize,
    /// How many full cycles remain. `0` = exhausted.
    cycles_remaining: u32,
}

impl SineTableSource {
    /// Construct a source that plays `samples` looped `cycles` times.
    /// `cycles == 0` or empty `samples` yields an immediately-exhausted
    /// source.
    #[must_use]
    pub const fn new(samples: &'static [i16], cycles: u32) -> Self {
        Self {
            samples,
            cursor: 0,
            cycles_remaining: cycles,
        }
    }
}

impl AudioSource for SineTableSource {
    fn fill(&mut self, buf: &mut [i16]) -> usize {
        if self.samples.is_empty() || self.cycles_remaining == 0 {
            return 0;
        }
        let mut written = 0;
        while written < buf.len() && self.cycles_remaining > 0 {
            let cycle_remaining = self.samples.len() - self.cursor;
            let take = cycle_remaining.min(buf.len() - written);
            buf[written..written + take]
                .copy_from_slice(&self.samples[self.cursor..self.cursor + take]);
            written += take;
            self.cursor += take;
            if self.cursor >= self.samples.len() {
                self.cursor = 0;
                self.cycles_remaining -= 1;
            }
        }
        written
    }

    fn len_hint(&self) -> Option<usize> {
        if self.samples.is_empty() || self.cycles_remaining == 0 {
            return Some(0);
        }
        let in_progress = self.samples.len() - self.cursor;
        // Saturating because `cycles_remaining` is `u32` and the slice
        // length is `usize`; on a 32-bit target the product could
        // overflow for a pathologically long cycle * count combination
        // (no real catalog hits this but the type contract is honest).
        let extra_cycles = self.cycles_remaining.saturating_sub(1) as usize;
        Some(in_progress.saturating_add(extra_cycles.saturating_mul(self.samples.len())))
    }
}

/// Plays a sequence of [`SineTableSource`]s back-to-back. Used for
/// multi-segment SFX (two-tone chirps, beep-gap-beep alerts).
///
/// Vec-backed because catalog segments range from 1 to 3; alloc is
/// already a baseline assumption of this crate.
#[derive(Debug)]
pub struct SineSequence {
    /// Segments in playback order. Drained from index `0` upward.
    segments: Vec<SineTableSource>,
    /// Index of the segment currently playing.
    idx: usize,
}

impl SineSequence {
    /// Construct a sequence from a vector of sine segments.
    #[must_use]
    pub const fn new(segments: Vec<SineTableSource>) -> Self {
        Self { segments, idx: 0 }
    }
}

impl AudioSource for SineSequence {
    fn fill(&mut self, buf: &mut [i16]) -> usize {
        let mut written = 0;
        while written < buf.len() {
            let Some(segment) = self.segments.get_mut(self.idx) else {
                break;
            };
            let n = segment.fill(&mut buf[written..]);
            if n == 0 {
                self.idx += 1;
                continue;
            }
            written += n;
        }
        written
    }

    fn len_hint(&self) -> Option<usize> {
        let mut total = 0usize;
        for (i, seg) in self.segments.iter().enumerate() {
            if i < self.idx {
                continue;
            }
            total = total.saturating_add(seg.len_hint()?);
        }
        Some(total)
    }
}

// =====================================================================
// SFX catalog — one builder per non-verbal `PhraseId` variant.
// =====================================================================
//
// Cycle counts mirror the durations the firmware previously baked:
// `100 cycles × 16 samples @ 16 kHz = 100 ms`.

/// 100 ms of 1 kHz. Voice-wake confirmation.
fn wake_chirp() -> SineSequence {
    SineSequence::new(vec![SineTableSource::new(SINE_1KHZ, 100)])
}

/// 50 ms of 2 kHz then 50 ms of 4 kHz — upward sweep on pickup.
fn pickup_chirp() -> SineSequence {
    SineSequence::new(vec![
        SineTableSource::new(SINE_2KHZ, 100),
        SineTableSource::new(SINE_4KHZ, 200),
    ])
}

/// 50 ms of 4 kHz — sharp single-tone startle.
fn startle_chirp() -> SineSequence {
    SineSequence::new(vec![SineTableSource::new(SINE_4KHZ, 200)])
}

/// 100 ms of 2 kHz, 80 ms silence, 100 ms of 2 kHz — two-pulse alert.
fn low_battery_chirp() -> SineSequence {
    SineSequence::new(vec![
        SineTableSource::new(SINE_2KHZ, 200),
        SineTableSource::new(SILENCE_8, 160),
        SineTableSource::new(SINE_2KHZ, 200),
    ])
}

/// 50 ms of 1 kHz then 80 ms of 2 kHz — upward "doot-DEE."
fn camera_mode_entered_chirp() -> SineSequence {
    SineSequence::new(vec![
        SineTableSource::new(SINE_1KHZ, 50),
        SineTableSource::new(SINE_2KHZ, 160),
    ])
}

/// 80 ms of 2 kHz then 50 ms of 1 kHz — downward "DEE-doot."
fn camera_mode_exited_chirp() -> SineSequence {
    SineSequence::new(vec![
        SineTableSource::new(SINE_2KHZ, 160),
        SineTableSource::new(SINE_1KHZ, 50),
    ])
}

// =====================================================================
// Verbal-phrase PCM — embedded at compile time via `include_bytes!`.
// =====================================================================
//
// Files are produced by `just bake-tts` (see `assets/README.md`) and
// committed alongside the source. Format: raw 16 kHz / 16-bit signed
// / mono / little-endian — matches the firmware I²S configuration so
// playback is decode-free.

/// Embed a `.pcm` asset and return it as a `&'static [u8]`. Wrapper
/// macro keeps the per-phrase arms in [`pcm_for`] readable.
macro_rules! pcm_asset {
    ($locale:literal, $phrase:literal) => {
        include_bytes!(concat!("../assets/", $locale, "/", $phrase, ".pcm")).as_slice()
    };
}

/// Resolve `(locale, phrase)` to the embedded raw-PCM bytes, or `None`
/// for verbal phrases not yet baked in this locale.
#[allow(
    clippy::match_same_arms,
    reason = "each arm's `pcm_asset!` expansion embeds a different file; clippy reads the macro shape as identical"
)]
const fn pcm_for(locale: Locale, phrase: PhraseId) -> Option<&'static [u8]> {
    match (locale, phrase) {
        (Locale::En, PhraseId::Greeting) => Some(pcm_asset!("en", "greeting")),
        (Locale::En, PhraseId::AcknowledgeName) => Some(pcm_asset!("en", "acknowledge_name")),
        (Locale::En, PhraseId::BatteryLow) => Some(pcm_asset!("en", "battery_low")),
        (Locale::Ja, PhraseId::Greeting) => Some(pcm_asset!("ja", "greeting")),
        (Locale::Ja, PhraseId::AcknowledgeName) => Some(pcm_asset!("ja", "acknowledge_name")),
        (Locale::Ja, PhraseId::BatteryLow) => Some(pcm_asset!("ja", "battery_low")),
        _ => None,
    }
}

/// PCM source backed by raw 16-bit LE bytes embedded via
/// `include_bytes!`. Reads the byte slice as i16 little-endian and
/// emits samples directly into the caller's buffer.
///
/// Shipped silent-stub `.pcm` files (committed before the user runs
/// `just bake-tts`) play as digital silence, not as `AssetMissing` —
/// keeping the firmware build end-to-end runnable.
#[derive(Debug)]
pub struct PcmSource {
    /// Raw little-endian i16 byte stream from `include_bytes!`. Two
    /// bytes per sample.
    bytes: &'static [u8],
    /// Index of the next byte to consume.
    cursor: usize,
}

impl PcmSource {
    /// Construct a PCM source over `bytes`. `bytes.len()` should be
    /// even (i16-aligned); a trailing odd byte is ignored.
    #[must_use]
    pub const fn new(bytes: &'static [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }
}

impl AudioSource for PcmSource {
    fn fill(&mut self, buf: &mut [i16]) -> usize {
        let mut written = 0;
        while written < buf.len() && self.cursor + 1 < self.bytes.len() {
            let lo = self.bytes[self.cursor];
            let hi = self.bytes[self.cursor + 1];
            buf[written] = i16::from_le_bytes([lo, hi]);
            self.cursor += 2;
            written += 1;
        }
        written
    }

    fn len_hint(&self) -> Option<usize> {
        Some((self.bytes.len().saturating_sub(self.cursor)) / 2)
    }
}

// =====================================================================
// BakedBackend
// =====================================================================

/// Speech backend that renders [`SpeechContent::Phrase`] entries from
/// compile-time tables. Stateless — instantiate once at boot, register
/// with the firmware speech router.
#[derive(Debug, Default, Clone, Copy)]
pub struct BakedBackend;

impl BakedBackend {
    /// Construct an instance. Stateless; call once at boot.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SpeechBackend for BakedBackend {
    fn name(&self) -> &'static str {
        "Baked"
    }

    fn can_handle(&self, content: &SpeechContent) -> bool {
        matches!(content, SpeechContent::Phrase(_))
    }

    fn render(&self, utterance: &Utterance) -> Result<Box<dyn AudioSource>, RenderError> {
        let SpeechContent::Phrase(phrase) = utterance.content else {
            return Err(RenderError::UnsupportedContent);
        };
        // `PhraseId` is `#[non_exhaustive]`, so the catch-all `_` arm
        // at the bottom is required syntactically. There is no stable
        // way to also force every currently-known variant to appear
        // above it — a new `PhraseId` variant added in core compiles
        // here and silently routes to `UnsupportedContent` until a
        // catalog arm is added. The exhaustiveness pin is the
        // `every_known_phrase_id_has_a_render_arm` test below; run
        // `cargo test -p stackchan-tts` after adding a variant.
        match phrase {
            // Non-verbal SFX: rendered from sine-cycle tables.
            PhraseId::WakeChirp => Ok(Box::new(wake_chirp())),
            PhraseId::PickupChirp => Ok(Box::new(pickup_chirp())),
            PhraseId::StartleChirp => Ok(Box::new(startle_chirp())),
            PhraseId::LowBatteryChirp => Ok(Box::new(low_battery_chirp())),
            PhraseId::CameraModeEnteredChirp => Ok(Box::new(camera_mode_entered_chirp())),
            PhraseId::CameraModeExitedChirp => Ok(Box::new(camera_mode_exited_chirp())),
            // Verbal phrases: served from compile-time-baked PCM. A
            // committed silent-stub plays as audible silence until the
            // user runs `just bake-tts`; truly-missing locale × phrase
            // combinations fall through to `AssetMissing`.
            PhraseId::Greeting | PhraseId::AcknowledgeName | PhraseId::BatteryLow => {
                pcm_for(utterance.locale, phrase)
                    .map(|bytes| Box::new(PcmSource::new(bytes)) as Box<dyn AudioSource>)
                    .ok_or(RenderError::AssetMissing)
            }
            // PhraseId is `#[non_exhaustive]`; new variants must
            // declare their treatment here before the catalog
            // supports them.
            _ => Err(RenderError::UnsupportedContent),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    clippy::panic
)]
mod tests {
    use super::*;
    use stackchan_core::voice::{ContentRef, Utterance};

    /// Drain an [`AudioSource`] fully into a Vec for size assertions. Caps
    /// at 100 k samples to avoid runaway tests.
    fn drain(mut src: Box<dyn AudioSource>) -> Vec<i16> {
        let mut all = Vec::new();
        let mut buf = [0_i16; 256];
        for _ in 0..400 {
            let n = src.fill(&mut buf);
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        all
    }

    #[test]
    fn sine_table_source_yields_expected_sample_count() {
        // 1 kHz cycle = 16 samples; 3 cycles = 48 samples.
        let mut src = SineTableSource::new(SINE_1KHZ, 3);
        let mut buf = [0_i16; 64];
        let n = src.fill(&mut buf);
        assert_eq!(n, 48, "3 cycles × 16 samples");
        assert_eq!(src.fill(&mut buf), 0, "exhausted after full drain");
    }

    #[test]
    fn sine_table_source_handles_partial_fill() {
        let mut src = SineTableSource::new(SINE_1KHZ, 3);
        let mut buf = [0_i16; 10];
        // Three 10-sample fills consume 30 of the 48 available; the
        // fourth pulls the remaining 18.
        assert_eq!(src.fill(&mut buf), 10);
        assert_eq!(src.fill(&mut buf), 10);
        assert_eq!(src.fill(&mut buf), 10);
        let mut tail = [0_i16; 32];
        assert_eq!(src.fill(&mut tail), 18);
        assert_eq!(src.fill(&mut tail), 0);
    }

    #[test]
    fn empty_table_or_zero_cycles_exhausts_immediately() {
        let mut buf = [0_i16; 4];
        assert_eq!(SineTableSource::new(&[], 100).fill(&mut buf), 0);
        assert_eq!(SineTableSource::new(SINE_1KHZ, 0).fill(&mut buf), 0);
    }

    #[test]
    fn sine_sequence_chains_segments_with_no_gap() {
        let seq = SineSequence::new(vec![
            SineTableSource::new(SINE_1KHZ, 1), // 16 samples
            SineTableSource::new(SINE_2KHZ, 1), // 8 samples
        ]);
        let pcm = drain(Box::new(seq));
        assert_eq!(pcm.len(), 24);
    }

    #[test]
    fn baked_backend_handles_phrase_content_only() {
        let b = BakedBackend::new();
        assert!(b.can_handle(&SpeechContent::Phrase(PhraseId::WakeChirp)));
        let dyn_ref = ContentRef::new(1).expect("non-zero");
        assert!(!b.can_handle(&SpeechContent::Dynamic(dyn_ref)));
    }

    #[test]
    fn baked_backend_renders_each_sfx_phrase() {
        let b = BakedBackend::new();
        for phrase in [
            PhraseId::WakeChirp,
            PhraseId::PickupChirp,
            PhraseId::StartleChirp,
            PhraseId::LowBatteryChirp,
            PhraseId::CameraModeEnteredChirp,
            PhraseId::CameraModeExitedChirp,
        ] {
            let src = b
                .render(&Utterance::phrase(phrase))
                .expect("SFX phrase must render");
            let pcm = drain(src);
            assert!(!pcm.is_empty(), "{phrase:?} produced empty AudioSource");
        }
    }

    #[test]
    fn baked_backend_renders_each_verbal_phrase_in_each_locale() {
        // 200 ms of 16 kHz mono i16 PCM = 3200 samples = 6400 bytes,
        // which matches the silent-stub size committed alongside the
        // manifest. A regenerated bake will be longer; a regression
        // that re-commits a 2-byte placeholder will fail this check.
        const MIN_SAMPLES: usize = 3200;
        let b = BakedBackend::new();
        for phrase in [
            PhraseId::Greeting,
            PhraseId::AcknowledgeName,
            PhraseId::BatteryLow,
        ] {
            for locale in [Locale::En, Locale::Ja] {
                let utterance = Utterance::phrase(phrase).with_locale(locale);
                let src = b
                    .render(&utterance)
                    .expect("verbal phrase must resolve to a baked-PCM source");
                let pcm = drain(src);
                assert!(
                    pcm.len() >= MIN_SAMPLES,
                    "{phrase:?}/{locale:?}: PCM is {} samples, expected ≥ {MIN_SAMPLES} \
                     — regenerate via `just bake-tts` or check the silent-stub size",
                    pcm.len(),
                );
            }
        }
    }

    #[test]
    fn every_known_phrase_id_has_a_render_arm() {
        // Explicit list of every `PhraseId` variant. The
        // `#[non_exhaustive]` attribute on the enum prevents
        // compile-time exhaustiveness checking on `BakedBackend::render`'s
        // match (a catch-all `_` arm is required), so this test is
        // the contract: every known variant must produce either a
        // valid AudioSource or a deliberate non-`UnsupportedContent`
        // error. When adding a `PhraseId` variant, also extend this
        // list and the corresponding render arm.
        const KNOWN: &[PhraseId] = &[
            PhraseId::WakeChirp,
            PhraseId::PickupChirp,
            PhraseId::StartleChirp,
            PhraseId::LowBatteryChirp,
            PhraseId::CameraModeEnteredChirp,
            PhraseId::CameraModeExitedChirp,
            PhraseId::Greeting,
            PhraseId::AcknowledgeName,
            PhraseId::BatteryLow,
        ];
        let b = BakedBackend::new();
        for phrase in KNOWN {
            // `UnsupportedContent` means the variant fell through the
            // catch-all in `render` — that's the bug we're pinning
            // against. Anything else (Ok, AssetMissing for unbaked
            // verbal phrases, BackendUnavailable) is fine.
            if matches!(
                b.render(&Utterance::phrase(*phrase)),
                Err(RenderError::UnsupportedContent)
            ) {
                panic!(
                    "{phrase:?} fell through to the catch-all arm — \
                     add a render arm in BakedBackend::render"
                );
            }
        }
    }

    #[test]
    fn baked_backend_rejects_dynamic_content() {
        let b = BakedBackend::new();
        let dyn_ref = ContentRef::new(42).expect("non-zero");
        let utterance = Utterance {
            content: SpeechContent::Dynamic(dyn_ref),
            ..Utterance::phrase(PhraseId::Greeting)
        };
        match b.render(&utterance) {
            Err(RenderError::UnsupportedContent) => {}
            Err(other) => panic!("expected UnsupportedContent, got {other:?}"),
            Ok(_) => panic!("expected UnsupportedContent, got Ok"),
        }
    }

    #[test]
    fn pickup_chirp_length_matches_design() {
        // 100 cycles of 2 kHz (8 samples) + 200 cycles of 4 kHz (4 samples)
        // = 800 + 800 = 1600 samples = 100 ms at 16 kHz.
        let pcm = drain(Box::new(pickup_chirp()));
        assert_eq!(pcm.len(), 1600);
    }
}
