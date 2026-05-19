//! Speech backend trait — turns [`Utterance`]s into [`AudioSource`]s.
//!
//! Backends register with the firmware speech router; the router
//! pattern-matches on [`Utterance::content`] and forwards to the first
//! backend whose [`SpeechBackend::can_handle`] returns `true`. Multiple
//! backends are expected to coexist (e.g. a `BakedBackend` for canned
//! [`SpeechContent::Phrase`]s + a future `CloudBackend` for
//! [`SpeechContent::Dynamic`]).
//!
//! [`Utterance`]: stackchan_core::voice::Utterance
//! [`SpeechContent::Phrase`]: stackchan_core::voice::SpeechContent::Phrase
//! [`SpeechContent::Dynamic`]: stackchan_core::voice::SpeechContent::Dynamic
//! [`Utterance::content`]: stackchan_core::voice::Utterance::content

use alloc::boxed::Box;
use stackchan_core::voice::{SpeechContent, Utterance};

use crate::source::AudioSource;

/// Reasons rendering an [`Utterance`] can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    /// The backend's [`SpeechBackend::can_handle`] returned `true` but
    /// it can't actually produce audio for this specific utterance —
    /// e.g. an unknown [`SpeechContent::Dynamic`] handle, or a
    /// [`PhraseId`] variant the backend hasn't been baked for yet.
    ///
    /// [`PhraseId`]: stackchan_core::voice::PhraseId
    /// [`SpeechContent::Dynamic`]: stackchan_core::voice::SpeechContent::Dynamic
    UnsupportedContent,
    /// A required asset (PCM file, network response, etc.) is
    /// unavailable. Distinct from [`Self::UnsupportedContent`] in that
    /// the content kind is supported in principle.
    AssetMissing,
    /// The backend itself is in a degraded state — Wi-Fi down for a
    /// cloud backend, decoder failed to initialize, etc. Caller may
    /// retry later.
    BackendUnavailable,
}

/// A pluggable speech-rendering backend.
///
/// `Send` because the speech router task may dispatch across embassy
/// task boundaries; backends that can't be sent should be wrapped in
/// the appropriate sync primitive at the registration site.
pub trait SpeechBackend: Send {
    /// Stable name for diagnostics / logging.
    fn name(&self) -> &'static str;

    /// Whether this backend can render `content`. Called by the router
    /// in registration order; the first `true` wins. Backends should
    /// answer cheaply — no I/O, no synthesis — since this gates
    /// dispatch.
    fn can_handle(&self, content: &SpeechContent) -> bool;

    /// Build an [`AudioSource`] that renders `utterance`.
    ///
    /// Called only when [`Self::can_handle`] returned `true` for the
    /// utterance's content. May still fail (asset gone, network
    /// down) — those paths return [`RenderError`].
    ///
    /// # Errors
    ///
    /// See [`RenderError`] variants.
    fn render(&self, utterance: &Utterance) -> Result<Box<dyn AudioSource>, RenderError>;
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use stackchan_core::voice::{ContentRef, PhraseId};

    struct StubBackend;

    impl SpeechBackend for StubBackend {
        fn name(&self) -> &'static str {
            "Stub"
        }

        fn can_handle(&self, content: &SpeechContent) -> bool {
            matches!(content, SpeechContent::Phrase(_))
        }

        fn render(&self, _utterance: &Utterance) -> Result<Box<dyn AudioSource>, RenderError> {
            Err(RenderError::UnsupportedContent)
        }
    }

    #[test]
    fn can_handle_dispatches_by_content_kind() {
        let b = StubBackend;
        assert!(b.can_handle(&SpeechContent::Phrase(PhraseId::Greeting)));
        let dyn_ref = ContentRef::new(1).expect("non-zero");
        assert!(!b.can_handle(&SpeechContent::Dynamic(dyn_ref)));
    }

    #[test]
    fn name_is_stable_diagnostic_string() {
        // `name` is used in defmt log lines when the router picks a
        // backend; pin it so log greps stay valid across refactors.
        let b = StubBackend;
        assert_eq!(b.name(), "Stub");
    }

    #[test]
    fn render_surfaces_typed_errors() {
        // The StubBackend always returns UnsupportedContent — exercise
        // the dispatch + the RenderError variant flow. Real backends
        // return Box<dyn AudioSource> on success; the router only
        // calls `render` after `can_handle` returned true, so this
        // path is the standard "I claimed I could but I really
        // can't" fallthrough that backends use to ask the router
        // to try the next registered backend.
        let b = StubBackend;
        let utt = Utterance::phrase(PhraseId::Greeting);
        let result = b.render(&utt);
        assert_eq!(result.err(), Some(RenderError::UnsupportedContent));
    }

    #[test]
    fn render_error_variants_are_distinct_and_copy() {
        // Pin that the three variants are equal only to themselves —
        // the router compares variants to decide whether to retry
        // (AssetMissing) vs fall through to the next backend
        // (UnsupportedContent) vs surface to the caller
        // (BackendUnavailable). `#[derive(Copy)]` lets a backend
        // return-by-value without cloning.
        assert_ne!(RenderError::UnsupportedContent, RenderError::AssetMissing);
        assert_ne!(RenderError::AssetMissing, RenderError::BackendUnavailable);
        assert_ne!(
            RenderError::UnsupportedContent,
            RenderError::BackendUnavailable
        );
        // Copy: assignment must not move.
        let e = RenderError::AssetMissing;
        let copied = e;
        assert_eq!(e, copied);
    }
}
