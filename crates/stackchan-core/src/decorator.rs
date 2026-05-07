//! Decorator overlays — small symbolic shapes drawn on top of the face
//! to amplify whatever emotion is showing.
//!
//! The face renders the emotion-driven base layer (eyes, mouth, blush);
//! the decorator layer paints one symbolic overlay on top — Heart for
//! affection, Sweat for distress, Dizzy stars for confusion — anchored
//! at fixed positions above the eyes. Only one decorator shows at a
//! time; per-tick trigger modifiers in [`crate::director::Phase::Decoration`]
//! decide which (if any) is active.
//!
//! ## Lifecycle
//!
//! Decorators are time-bounded: each carries an `expires_at` `Instant`
//! and the [`crate::modifiers::DecoratorExpiry`] modifier clears the
//! field when the deadline passes. A trigger modifier that fires while
//! one is already active just overwrites the field — there's no
//! priority arbitration beyond the modifier sort order.

use crate::clock::Instant;

/// Decorator overlay kind. Picked by trigger modifiers in
/// [`crate::director::Phase::Decoration`]; rendered by the draw code on
/// top of the emotion base layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Decorator {
    /// Pink heart cluster — affection / love overlay.
    Heart,
    /// Light-blue tear drop — distress / surprise overlay.
    Sweat,
    /// Three-dot arc — dizzy / shaken overlay.
    Dizzy,
    /// Cupped-hand / ear shape — listening overlay. Armed by
    /// [`crate::modifiers::DecoratorFromListening`] while
    /// [`crate::Intent::Listening`] holds (operator-triggered or
    /// wake-word triggered).
    Ear,
    /// Concentric blue rings — ESP-NOW pairing window active. Set by
    /// [`crate::modifiers::RemoteCommandModifier`] for the duration of
    /// an [`crate::RemoteCommand::EnterPairing`] hold.
    Pairing,
}

impl Decorator {
    /// Lowercase wire name for the HTTP `/state` snapshot. Mirrors
    /// [`crate::Emotion::wire_str`]'s convention so a consumer can
    /// lift a decorator string off `/state` and post it back without
    /// case translation (when a future write route ships).
    ///
    /// Exhaustive without a wildcard — adding a variant forces a
    /// conscious choice of wire string.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Heart => "heart",
            Self::Sweat => "sweat",
            Self::Dizzy => "dizzy",
            Self::Ear => "ear",
            Self::Pairing => "pairing",
        }
    }

    /// Single-byte wire encoding for any future BLE GATT exposure.
    /// Append-only, mirroring [`crate::Emotion::wire_byte`].
    #[must_use]
    pub const fn wire_byte(self) -> u8 {
        match self {
            Self::Heart => 0,
            Self::Sweat => 1,
            Self::Dizzy => 2,
            Self::Ear => 3,
            Self::Pairing => 4,
        }
    }

    /// Every variant in declaration order. Manually maintained — the
    /// exhaustive match arms in [`Self::wire_byte`] / [`Self::wire_str`]
    /// are the compile-time guard; this slice has no compile-time
    /// completeness check, so the `all_length_matches_variant_count`
    /// test is the trip-wire that forces an update on variant addition.
    pub const ALL: &'static [Self] = &[
        Self::Heart,
        Self::Sweat,
        Self::Dizzy,
        Self::Ear,
        Self::Pairing,
    ];
}

/// Active decorator overlay with its expiry deadline.
///
/// The field is `Option<DecoratorState>` on [`crate::face::Face`];
/// `None` is the steady state. Trigger modifiers fill it in; the
/// expiry modifier clears it when `expires_at` passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoratorState {
    /// Which decorator is showing.
    pub kind: Decorator,
    /// Wall-clock instant at which this decorator stops being drawn.
    pub expires_at: Instant,
}

impl DecoratorState {
    /// Construct a state that holds `kind` from `now` for `duration_ms`
    /// milliseconds.
    #[must_use]
    pub const fn hold_for(kind: Decorator, now: Instant, duration_ms: u64) -> Self {
        Self {
            kind,
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

    /// Length pin for [`Decorator::ALL`]. Mirrors
    /// `Emotion::all_length_matches_variant_count` — the slice has no
    /// compile-time exhaustiveness check, so the count is the trip-wire.
    #[test]
    fn all_length_matches_variant_count() {
        assert_eq!(
            Decorator::ALL.len(),
            5,
            "update Decorator::ALL when adding a variant"
        );
    }

    #[test]
    fn wire_byte_mapping_is_stable() {
        assert_eq!(Decorator::Heart.wire_byte(), 0);
        assert_eq!(Decorator::Sweat.wire_byte(), 1);
        assert_eq!(Decorator::Dizzy.wire_byte(), 2);
        assert_eq!(Decorator::Ear.wire_byte(), 3);
        assert_eq!(Decorator::Pairing.wire_byte(), 4);
    }

    #[test]
    fn wire_strings_are_unique_and_lowercase() {
        for (i, &a) in Decorator::ALL.iter().enumerate() {
            let wa = a.wire_str();
            assert!(wa.chars().all(|c| !c.is_uppercase()));
            for &b in &Decorator::ALL[i + 1..] {
                assert_ne!(wa, b.wire_str());
            }
        }
    }

    #[test]
    fn hold_for_pins_expiry_at_now_plus_duration() {
        let now = Instant::from_millis(1_000);
        let s = DecoratorState::hold_for(Decorator::Heart, now, 2_000);
        assert_eq!(s.kind, Decorator::Heart);
        assert_eq!(s.expires_at, Instant::from_millis(3_000));
    }

    #[test]
    fn is_expired_at_or_after_deadline() {
        let now = Instant::from_millis(1_000);
        let s = DecoratorState::hold_for(Decorator::Sweat, now, 500);
        assert!(!s.is_expired(Instant::from_millis(1_000)));
        assert!(!s.is_expired(Instant::from_millis(1_499)));
        assert!(s.is_expired(Instant::from_millis(1_500)));
        assert!(s.is_expired(Instant::from_millis(2_000)));
    }
}
