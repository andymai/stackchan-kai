//! Operator-selected baseline for the avatar's energy level.
//!
//! [`Mood`] is a *user* setting (set via dashboard / HTTP / SD config),
//! distinct from [`crate::Emotion`] which is *reactive* (driven by
//! sensors and the autonomous cycler). The two compose: emotion picks
//! the visible expression palette, then mood scales the modulators
//! that drive cadence and energy (blink rate, breath depth, idle
//! drift) so the same Happy face can read as `Playful` (fast blinks,
//! shallow breath) or `Calm` (slow blinks, deep breath).
//!
//! ## Why a separate enum (not just a per-emotion multiplier)
//!
//! Emotion changes second-to-second; mood is a property of the room
//! the avatar is in. Folding the two would force every reactive
//! emotion-driver to know about user preferences. Splitting them lets
//! `StyleFromMood` run *after* `StyleFromEmotion` and apply a stable
//! multiplier on top, with no cross-talk to the reactive side.

/// The operator-selected energy baseline.
///
/// Set via persisted config (`STACKCHAN.RON`) and the HTTP control
/// plane; never written by reactive modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Mood {
    /// Default. Modifier weights pass through unchanged — emotion
    /// drives cadence with no additional multiplier on top.
    #[default]
    Neutral,
    /// Slow + deep. Lower blink rate, deeper breath, less idle drift.
    /// Reads as serene / meditative.
    Calm,
    /// Fast + light. Higher blink rate, shallower breath, more idle
    /// drift. Reads as alert / engaged.
    Playful,
    /// Minimal extraneous motion. Lower blink rate, slightly shallower
    /// breath, very little idle drift. Reads as concentrated / locked-in.
    Focus,
    /// Drowsy. Very low blink rate, very deep slow breath, minimal
    /// drift. Reads as on-the-edge-of-sleep — distinct from
    /// `Emotion::Sleepy` because it stays set across emotion changes.
    Sleepy,
}

impl Mood {
    /// Wire string for the HTTP `/state` snapshot and `/settings`
    /// payload. Lowercase, mirrors [`crate::Emotion::wire_str`].
    ///
    /// Exhaustive without a wildcard: adding a variant forces a
    /// conscious choice of wire string here.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Calm => "calm",
            Self::Playful => "playful",
            Self::Focus => "focus",
            Self::Sleepy => "sleepy",
        }
    }

    /// Inverse of [`Self::wire_str`]. Returns `None` on any string
    /// that isn't an exact lowercase match. Mirrors
    /// [`crate::Palette::from_wire_str`].
    #[must_use]
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "neutral" => Some(Self::Neutral),
            "calm" => Some(Self::Calm),
            "playful" => Some(Self::Playful),
            "focus" => Some(Self::Focus),
            "sleepy" => Some(Self::Sleepy),
            _ => None,
        }
    }

    /// Multiplier applied to `face.style.blink_rate_scale` after
    /// [`crate::modifiers::StyleFromEmotion`] has set the per-emotion
    /// baseline. `1.0` = no change; `< 1.0` slows blinks, `> 1.0`
    /// speeds them up.
    #[must_use]
    pub const fn blink_multiplier(self) -> f32 {
        match self {
            Self::Neutral => 1.0,
            Self::Calm => 0.7,
            Self::Playful => 1.4,
            Self::Focus => 0.6,
            Self::Sleepy => 0.4,
        }
    }

    /// Multiplier applied to `face.style.breath_depth_scale`. Same
    /// convention as [`Self::blink_multiplier`].
    #[must_use]
    pub const fn breath_multiplier(self) -> f32 {
        match self {
            Self::Neutral => 1.0,
            Self::Calm => 1.2,
            Self::Playful => 0.9,
            // `Focus` is *shallower* than `Playful` — held breath while
            // concentrating, in contrast to `Playful`'s lighter bouncy
            // breath. Distinct value so clippy's match-same-arms catches
            // a future palette tweak that would collapse them.
            Self::Focus => 0.85,
            Self::Sleepy => 1.4,
        }
    }

    /// Multiplier applied to idle-drift amplitude (eye centre jitter
    /// and idle head glances). `1.0` = baseline; `< 1.0` damps the
    /// random drift.
    #[must_use]
    pub const fn drift_multiplier(self) -> f32 {
        match self {
            Self::Neutral => 1.0,
            Self::Calm => 0.7,
            Self::Playful => 1.3,
            Self::Focus => 0.5,
            Self::Sleepy => 0.4,
        }
    }

    /// Every variant in declaration order. Manually maintained;
    /// `all_length_matches_variant_count` is the trip-wire that forces
    /// an update on variant addition. (See [`crate::Emotion::ALL`] for
    /// the same pattern.)
    pub const ALL: &'static [Self] = &[
        Self::Neutral,
        Self::Calm,
        Self::Playful,
        Self::Focus,
        Self::Sleepy,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Length pin for [`Mood::ALL`]. Mirrors `Emotion::ALL`'s trip-wire.
    #[test]
    fn all_length_matches_variant_count() {
        assert_eq!(Mood::ALL.len(), 5, "update Mood::ALL when adding a variant");
    }

    #[test]
    fn neutral_passes_through_unchanged() {
        assert!((Mood::Neutral.blink_multiplier() - 1.0).abs() < f32::EPSILON);
        assert!((Mood::Neutral.breath_multiplier() - 1.0).abs() < f32::EPSILON);
        assert!((Mood::Neutral.drift_multiplier() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn playful_blinks_faster_than_calm() {
        assert!(Mood::Playful.blink_multiplier() > Mood::Calm.blink_multiplier());
    }

    #[test]
    fn calm_breath_deeper_than_playful() {
        assert!(Mood::Calm.breath_multiplier() > Mood::Playful.breath_multiplier());
    }

    #[test]
    fn focus_minimises_drift_compared_to_neutral() {
        assert!(Mood::Focus.drift_multiplier() < Mood::Neutral.drift_multiplier());
    }

    #[test]
    fn sleepy_blinks_slower_than_calm() {
        // Sleepy should be the slowest-blink mood — even slower than
        // Calm, because Sleepy stays set across emotion changes
        // whereas the Sleepy *emotion* is reactive.
        assert!(Mood::Sleepy.blink_multiplier() < Mood::Calm.blink_multiplier());
    }

    #[test]
    fn wire_strings_are_unique_and_lowercase() {
        for (i, &a) in Mood::ALL.iter().enumerate() {
            let wa = a.wire_str();
            assert!(wa.chars().all(|c| !c.is_uppercase()));
            for &b in &Mood::ALL[i + 1..] {
                assert_ne!(wa, b.wire_str());
            }
        }
    }

    #[test]
    fn wire_str_round_trip_for_all_variants() {
        // Pins the exact string each variant maps to. A future
        // rename in `wire_str` without an update here will trip
        // this rather than silently breaking persisted runtime
        // state and HTTP body parsers downstream.
        for &m in Mood::ALL {
            let s = m.wire_str();
            assert_eq!(Mood::from_wire_str(s), Some(m));
        }
    }

    #[test]
    fn from_wire_str_rejects_unknown() {
        assert_eq!(Mood::from_wire_str(""), None);
        assert_eq!(Mood::from_wire_str("NEUTRAL"), None); // case sensitive
        assert_eq!(Mood::from_wire_str("ecstatic"), None);
    }
}
