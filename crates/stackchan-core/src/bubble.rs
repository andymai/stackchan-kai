//! Speech-bubble overlay — short text rendered above the face for
//! soliloquy beats, voice-feedback hints, or operator-supplied notes.
//!
//! The bubble layer sits on top of the [`crate::face::Face`] base
//! layer (eyes, mouth, blush, decorator), drawn last so its rounded
//! rectangle and black text occlude the face geometry where they
//! overlap. Only one bubble shows at a time; trigger modifiers in
//! [`crate::director::Phase::Decoration`] (or short-circuit code paths
//! in firmware tasks) populate the field; [`crate::modifiers::BubbleExpiry`]
//! clears it on deadline.
//!
//! ## Storage shape
//!
//! [`BubbleState::text`] is `&'static str`. Embedded firmware avoids
//! per-tick `String` allocations on the render path; phrase pools are
//! the canonical content source. Dynamic text from external sources
//! (e.g. an MCP `speak` tool with a runtime-supplied string) needs a
//! separate mechanism — likely a small interning arena — rather than
//! widening this surface.
//!
//! ## Lifecycle
//!
//! Same pattern as [`crate::decorator::DecoratorState`]: each bubble
//! carries an `expires_at` `Instant` and the
//! [`crate::modifiers::BubbleExpiry`] modifier clears the field when
//! the deadline passes. A trigger that fires while one is already
//! active just overwrites the field — there's no priority arbitration
//! beyond modifier sort order.

use crate::clock::Instant;

/// Active speech-bubble overlay with its expiry deadline.
///
/// Stored as `Option<BubbleState>` on [`crate::face::Face`]; `None`
/// is the steady state. Trigger modifiers fill it in; the expiry
/// modifier clears it when `expires_at` passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BubbleState {
    /// Text content. `&'static str` so the render path stays
    /// allocation-free; firmware feeds this from a phrase pool.
    pub text: &'static str,
    /// Wall-clock instant at which this bubble stops being drawn.
    pub expires_at: Instant,
}

impl BubbleState {
    /// Construct a state that holds `text` from `now` for `duration_ms`
    /// milliseconds.
    #[must_use]
    pub const fn hold_for(text: &'static str, now: Instant, duration_ms: u64) -> Self {
        Self {
            text,
            expires_at: Instant::from_millis(now.as_millis() + duration_ms),
        }
    }

    /// `true` iff `now` has reached or passed [`Self::expires_at`].
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_for_pins_expiry_at_now_plus_duration() {
        let now = Instant::from_millis(1_000);
        let s = BubbleState::hold_for("hi", now, 2_000);
        assert_eq!(s.text, "hi");
        assert_eq!(s.expires_at, Instant::from_millis(3_000));
    }

    #[test]
    fn is_expired_at_or_after_deadline() {
        let now = Instant::from_millis(1_000);
        let s = BubbleState::hold_for("hi", now, 500);
        assert!(!s.is_expired(Instant::from_millis(1_000)));
        assert!(!s.is_expired(Instant::from_millis(1_499)));
        assert!(s.is_expired(Instant::from_millis(1_500)));
        assert!(s.is_expired(Instant::from_millis(2_000)));
    }

    #[test]
    fn equality_compares_text_and_expiry() {
        let now = Instant::from_millis(0);
        let a = BubbleState::hold_for("a", now, 1_000);
        let b = BubbleState::hold_for("a", now, 1_000);
        let c = BubbleState::hold_for("b", now, 1_000);
        let d = BubbleState::hold_for("a", now, 2_000);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }
}
