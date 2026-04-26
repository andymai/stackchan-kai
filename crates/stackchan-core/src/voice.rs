//! Speech I/O surface of the entity.
//!
//! Two parallel surfaces during the chirp → utterance transition:
//!
//! - [`Voice::chirp_request`] (legacy): one-shot non-verbal SFX
//!   ([`ChirpKind`]) the firmware audio task plays via fixed sine
//!   tables. Modifiers set; render task drains.
//! - [`Voice::utterance_request`] (current): structured speech intent
//!   carrying [`SpeechContent`] (canned [`PhraseId`] or out-of-band
//!   [`ContentRef`] handle), [`Locale`], [`SpeechStyle`], and [`Priority`].
//!   The `stackchan-tts` `SpeechBackend` resolves it to playable audio.
//!
//! The chirp surface is being subsumed into the utterance surface —
//! current chirps map onto non-verbal `PhraseId` variants (e.g.
//! [`PhraseId::WakeChirp`]). Once every modifier publishes `Utterance`
//! and the firmware router consumes both code paths, [`ChirpKind`] +
//! [`Voice::chirp_request`] are retired.
//!
//! [`Voice::is_speaking`] mirrors the firmware's `AUDIO_TX_PLAYING`
//! atomic for sound-reactive modifiers that need to suppress
//! self-trigger during own playback.

use core::num::NonZeroU32;

/// One-shot audio clips the firmware can enqueue.
///
/// Adding a variant requires:
/// 1. A matching enqueue helper in `stackchan-firmware/src/audio.rs`
///    (e.g. `try_enqueue_my_chirp()`).
/// 2. The render task's chirp dispatch arm in `main.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChirpKind {
    /// Pickup-detected chirp. Set by `EmotionFromIntent` on a transition
    /// into `Intent::PickedUp` (driven by the `Handling` skill).
    Pickup,
    /// Voice-wake chirp. Set by `EmotionFromVoice` when sustained audio
    /// wakes the entity.
    Wake,
    /// Startle chirp. Set by `IntentFromLoud` on a transient acoustic
    /// spike (clap, shout, slam) — distinct from `Wake`, which is
    /// sustained-voice driven.
    Startle,
    /// Low-battery alert beep. Set by `EmotionFromBattery` when the
    /// percent crosses the enter threshold while unplugged.
    LowBatteryAlert,
    /// Camera-mode-entered tone. Set by the firmware's camera-mode
    /// toggle handler.
    CameraModeEntered,
    /// Camera-mode-exited tone. Counterpart to [`Self::CameraModeEntered`].
    CameraModeExited,
}

/// Catalog of canned phrases the `SpeechBackend` knows how to render.
///
/// Two flavors share the enum:
///
/// - **Non-verbal SFX** (suffix `Chirp` / `Alert`): subsume the legacy
///   [`ChirpKind`] surface. The baked backend renders these as
///   fixed-cycle sine tables — no PCM bake required.
/// - **Verbal phrases**: rendered from raw 16 kHz / 16-bit mono PCM
///   committed under `crates/stackchan-tts/assets/<locale>/<phrase>.pcm`
///   and embedded via `include_bytes!`.
///
/// `#[non_exhaustive]` so adding a phrase isn't a breaking change for
/// downstream matchers — backends pattern-match exhaustively.
///
/// (The `SpeechBackend` trait lives in `stackchan-tts`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PhraseId {
    // ----- Non-verbal SFX (subsumes ChirpKind) -----
    /// 100 ms of 1 kHz sine. Voice-wake confirmation.
    WakeChirp,
    /// Two-tone upward sweep (2 kHz → 4 kHz). Pickup detected.
    PickupChirp,
    /// 50 ms of 4 kHz. Sharp startle reaction.
    StartleChirp,
    /// Two-pulse 2 kHz beep. Battery crossed alert threshold.
    LowBatteryChirp,
    /// Upward two-tone (1 kHz → 2 kHz). Camera mode entered.
    CameraModeEnteredChirp,
    /// Downward two-tone (2 kHz → 1 kHz). Camera mode exited.
    CameraModeExitedChirp,

    // ----- Verbal phrases -----
    /// Boot greeting ("Hello, I'm Stack-chan." / "こんにちは、スタックチャンです。").
    Greeting,
    /// Acknowledge being addressed by name.
    AcknowledgeName,
    /// Spoken low-battery notice (verbal counterpart to
    /// [`Self::LowBatteryChirp`]).
    BatteryLow,
}

/// Spoken locale for an [`Utterance`]. Selects the per-locale clip /
/// voice profile the `SpeechBackend` uses.
///
/// (The `SpeechBackend` trait lives in `stackchan-tts`.)
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Locale {
    /// English.
    #[default]
    En,
    /// Japanese.
    Ja,
}

/// Vocal style override for an [`Utterance`]. Backend-specific in
/// effect — baked clips ignore it; cloud / on-device synthesizers
/// pass it through to the engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VocalStyle {
    /// Backend default voice.
    #[default]
    Neutral,
    /// Upbeat / smiling delivery.
    Cheerful,
    /// Quiet / breathy.
    Whisper,
    /// Energetic / raised.
    Excited,
}

/// How an [`Utterance`] picks its vocal style. Defaults to
/// [`Self::FromEmotion`] so authors don't have to thread an explicit
/// style through every speech intent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechStyle {
    /// Backend derives style from `entity.mind.affect.emotion` at
    /// render time. The default; tracks emotion modifiers
    /// automatically.
    #[default]
    FromEmotion,
    /// Caller-specified vocal style, ignoring emotion.
    Vocal(VocalStyle),
}

/// Speech preemption / queueing rank. Higher variants preempt lower
/// in-flight speech (single-in-flight model — see
/// `stackchan-tts::SpeechRouter`).
///
/// `#[repr(u8)]` so the firmware can store `Priority as u8` in an
/// `AtomicU8` for the in-flight priority tracker — discriminant
/// assignment is `0..=3` in declaration order, matching the derived
/// `Ord` semantics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Priority {
    /// Idle chatter. Yields to anything else queued.
    Background = 0,
    /// Default for emotion-driven greetings, acknowledgements.
    #[default]
    Normal = 1,
    /// Status notifications (battery low spoken, mode change).
    Important = 2,
    /// Safety-critical (cannot be preempted; preempts everything else).
    Critical = 3,
}

/// Opaque handle to dynamic speech content registered out-of-band.
///
/// Core stays alloc-free; cloud / on-device backends that produce
/// speech from runtime text register the text with their own
/// firmware-side registry and publish a [`ContentRef`] handle in the
/// [`Utterance`]. The backend resolves handle → text → audio at
/// render time.
///
/// `NonZeroU32` so `Option<ContentRef>` is the same size as
/// `ContentRef` itself (niche optimization), and `0` is a clear
/// "uninitialized / sentinel" value if anyone tries to construct one
/// directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentRef(NonZeroU32);

impl ContentRef {
    /// Construct from a raw handle. Returns `None` for `0`.
    #[must_use]
    pub const fn new(id: u32) -> Option<Self> {
        match NonZeroU32::new(id) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// Raw handle value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Content of a speech [`Utterance`] — what to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechContent {
    /// Canned phrase from the catalog. The baked backend resolves
    /// these directly without runtime synthesis.
    Phrase(PhraseId),
    /// Dynamic content registered out-of-band. The backend that
    /// issued the [`ContentRef`] resolves it back to text / audio.
    Dynamic(ContentRef),
}

/// A speech request published by a modifier or skill.
///
/// Modifiers set `entity.voice.utterance_request = Some(Utterance::…)`
/// during their tick; the firmware speech router drains the field
/// after `Director::run` returns and dispatches to a registered
/// `SpeechBackend`.
///
/// (The `SpeechBackend` trait lives in `stackchan-tts`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Utterance {
    /// What to say.
    pub content: SpeechContent,
    /// Spoken locale.
    pub locale: Locale,
    /// Vocal style override (or follow emotion).
    pub style: SpeechStyle,
    /// Preemption rank.
    pub priority: Priority,
}

impl Utterance {
    /// Convenience constructor for a canned-phrase utterance with
    /// default locale / style / priority. Equivalent to:
    ///
    /// ```ignore
    /// Utterance {
    ///     content: SpeechContent::Phrase(id),
    ///     locale: Locale::default(),
    ///     style: SpeechStyle::default(),
    ///     priority: Priority::default(),
    /// }
    /// ```
    #[must_use]
    pub const fn phrase(id: PhraseId) -> Self {
        Self {
            content: SpeechContent::Phrase(id),
            locale: Locale::En,
            style: SpeechStyle::FromEmotion,
            priority: Priority::Normal,
        }
    }

    /// Set the locale on a builder-style chain.
    #[must_use]
    pub const fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    /// Set the style on a builder-style chain.
    #[must_use]
    pub const fn with_style(mut self, style: SpeechStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the priority on a builder-style chain.
    #[must_use]
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
}

/// The entity's outbound audio surface.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    /// Legacy one-shot chirp. Subsumed by [`Self::utterance_request`];
    /// retained until every modifier publishes `Utterance` and the
    /// firmware router consumes both code paths.
    pub chirp_request: Option<ChirpKind>,
    /// Speech intent published this frame. The router drains and
    /// clears it after `Director::run` returns.
    pub utterance_request: Option<Utterance>,
    /// Mirrors the firmware's `AUDIO_TX_PLAYING` atomic. Sound-reactive
    /// modifiers (`EmotionFromVoice`, `IntentFromLoud`) read this to
    /// gate themselves during the entity's own speech, so they don't
    /// re-trigger on the speaker output.
    pub is_speaking: bool,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;

    #[test]
    fn content_ref_rejects_zero() {
        assert!(ContentRef::new(0).is_none());
        let cr = ContentRef::new(42).expect("non-zero");
        assert_eq!(cr.get(), 42);
    }

    #[test]
    fn utterance_phrase_defaults() {
        let u = Utterance::phrase(PhraseId::Greeting);
        assert_eq!(u.content, SpeechContent::Phrase(PhraseId::Greeting));
        assert_eq!(u.locale, Locale::En);
        assert!(matches!(u.style, SpeechStyle::FromEmotion));
        assert_eq!(u.priority, Priority::Normal);
    }

    #[test]
    fn utterance_builder_chains() {
        let u = Utterance::phrase(PhraseId::BatteryLow)
            .with_locale(Locale::Ja)
            .with_priority(Priority::Important)
            .with_style(SpeechStyle::Vocal(VocalStyle::Cheerful));
        assert_eq!(u.locale, Locale::Ja);
        assert_eq!(u.priority, Priority::Important);
        assert!(matches!(u.style, SpeechStyle::Vocal(VocalStyle::Cheerful)));
    }

    #[test]
    fn priority_orders_critical_above_background() {
        assert!(Priority::Critical > Priority::Important);
        assert!(Priority::Important > Priority::Normal);
        assert!(Priority::Normal > Priority::Background);
    }

    #[test]
    fn voice_default_is_silent() {
        let v = Voice::default();
        assert!(v.chirp_request.is_none());
        assert!(v.utterance_request.is_none());
        assert!(!v.is_speaking);
    }
}
