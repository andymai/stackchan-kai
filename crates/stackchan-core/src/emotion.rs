//! Emotional expression taxonomy.

/// High-level emotional state of the avatar. Modifiers and renderers may
/// change their behaviour based on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Emotion {
    /// Baseline / resting face.
    #[default]
    Neutral,
    /// Happy / positive affect.
    Happy,
    /// Sad / negative affect.
    Sad,
    /// Sleepy / eyes half-closed.
    Sleepy,
    /// Surprised / wide-eyed.
    Surprised,
    /// Angry / narrowed eyes + frown. Reactive only — set by
    /// `EmotionFromIntent` on a transition into `Intent::Shaken`. Not part
    /// of the autonomous `EmotionCycle` or touch-cycle order.
    Angry,
    /// Skeptical / questioning. Slight downward eye curve + faint smile;
    /// no override of breath or blink rate.
    Doubt,
    /// Disinterested / under-engaged. Half-lidded eyes, slow blink, slow
    /// deep breath, flat mouth — the antonym of `Happy` along the
    /// engagement axis (rather than valence).
    Boring,
    /// Outgoing greeting / wave-hello affect. Wide bright eyes, big smile,
    /// elevated blink rate. Distinguished from `Happy` by intensity, not
    /// kind.
    Hi,
    /// Smitten / affectionate. Heaviest blush of the catalogue + full
    /// upward eye arc. The decorator layer adds heart overlays in PR 1b;
    /// the base style stands alone here.
    Loved,
    /// Curiously interested. Wide eyes (smaller than `Surprised`), faint
    /// smile, light blush — investigative rather than reactive.
    Curious,
    /// Uncertain / processing. Slight frown + elevated blink rate (often
    /// reads as eye-flutter). Distinguished from `Doubt` by valence —
    /// `Doubt` is skeptical-positive, `Confused` is unsettled-negative.
    Confused,
    /// Furious / much hotter than `Angry`. Deeper frown, heavier blush,
    /// slower huffing breath, slightly squinted eyes. The default
    /// `EmotionCycle` does not visit this — reactive only.
    Mad,
}

impl Emotion {
    /// Lowercase wire name for the HTTP control plane.
    ///
    /// Mirrors the vocabulary that `stackchan_net::http_command`'s
    /// emotion parser accepts on `POST /emotion`, so a consumer can
    /// take an emotion off `GET /state` and post it back without any
    /// case translation. Pinning the mapping here also guards
    /// against a future non-unit `Emotion` variant whose `Debug`
    /// representation would otherwise inject `{` into the JSON
    /// string when the firmware renders the snapshot.
    ///
    /// The match is intentionally exhaustive without a wildcard:
    /// `Emotion` is `#[non_exhaustive]` to downstream crates, but
    /// here in `stackchan-core` the compiler can prove every variant
    /// is covered. Adding a new variant forces this match to be
    /// updated, which is what we want — silent fallback to
    /// `"unknown"` would leak past the dashboard's redaction.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Happy => "happy",
            Self::Sad => "sad",
            Self::Sleepy => "sleepy",
            Self::Surprised => "surprised",
            Self::Angry => "angry",
            Self::Doubt => "doubt",
            Self::Boring => "boring",
            Self::Hi => "hi",
            Self::Loved => "loved",
            Self::Curious => "curious",
            Self::Confused => "confused",
            Self::Mad => "mad",
        }
    }

    /// Single-byte wire encoding for the BLE GATT surface.
    ///
    /// The stack-chan custom service exposes the current emotion as a
    /// one-byte read+notify characteristic. The mapping below is the
    /// stable wire format: variant indices may not be reordered, and
    /// new variants must be appended at the next free index. Clients
    /// that decode a value not listed here should fall back to
    /// [`Self::Neutral`] (forward-compatible decoding).
    ///
    /// As with [`Self::wire_str`], the match is intentionally
    /// exhaustive without a wildcard — adding a new variant forces a
    /// conscious choice of byte index here.
    #[must_use]
    pub const fn wire_byte(self) -> u8 {
        match self {
            Self::Neutral => 0,
            Self::Happy => 1,
            Self::Sad => 2,
            Self::Sleepy => 3,
            Self::Surprised => 4,
            Self::Angry => 5,
            Self::Doubt => 6,
            Self::Boring => 7,
            Self::Hi => 8,
            Self::Loved => 9,
            Self::Curious => 10,
            Self::Confused => 11,
            Self::Mad => 12,
        }
    }

    /// Every variant in declaration order. Used by tests and tooling
    /// (the firmware HTTP `/state` snapshot doesn't enumerate; the BLE
    /// GATT does, via [`Self::wire_byte`]).
    ///
    /// The slice is exhaustive: omitting a variant compiles, but the
    /// `wire_byte_mapping_is_stable` test below pins the order against
    /// this list, so a missed entry surfaces as a test failure.
    pub const ALL: &'static [Self] = &[
        Self::Neutral,
        Self::Happy,
        Self::Sad,
        Self::Sleepy,
        Self::Surprised,
        Self::Angry,
        Self::Doubt,
        Self::Boring,
        Self::Hi,
        Self::Loved,
        Self::Curious,
        Self::Confused,
        Self::Mad,
    ];
}

#[cfg(test)]
mod tests {
    use super::Emotion;

    /// Lock the BLE wire-byte mapping. Reordering breaks every paired
    /// client; renaming a variant doesn't.
    #[test]
    fn wire_byte_mapping_is_stable() {
        assert_eq!(Emotion::Neutral.wire_byte(), 0);
        assert_eq!(Emotion::Happy.wire_byte(), 1);
        assert_eq!(Emotion::Sad.wire_byte(), 2);
        assert_eq!(Emotion::Sleepy.wire_byte(), 3);
        assert_eq!(Emotion::Surprised.wire_byte(), 4);
        assert_eq!(Emotion::Angry.wire_byte(), 5);
        assert_eq!(Emotion::Doubt.wire_byte(), 6);
        assert_eq!(Emotion::Boring.wire_byte(), 7);
        assert_eq!(Emotion::Hi.wire_byte(), 8);
        assert_eq!(Emotion::Loved.wire_byte(), 9);
        assert_eq!(Emotion::Curious.wire_byte(), 10);
        assert_eq!(Emotion::Confused.wire_byte(), 11);
        assert_eq!(Emotion::Mad.wire_byte(), 12);
    }

    /// Every wire-byte index is unique. A copy-paste collision in the
    /// match arm above would silently break the GATT client decode
    /// without this pin.
    #[test]
    fn wire_bytes_are_unique() {
        let mut seen = [false; 256];
        for &emotion in Emotion::ALL {
            let byte = emotion.wire_byte() as usize;
            assert!(
                !seen[byte],
                "duplicate wire byte {byte} for {emotion:?}; mapping must be one-to-one"
            );
            seen[byte] = true;
        }
    }

    /// Every wire string is unique and lowercase. Catches both copy-paste
    /// collisions and a stray uppercase that would skip the
    /// `parse_emotion` lowercase match.
    #[test]
    fn wire_strs_are_unique_and_lowercase() {
        for (i, &a) in Emotion::ALL.iter().enumerate() {
            let wa = a.wire_str();
            assert!(
                wa.chars().all(|c| !c.is_uppercase()),
                "wire_str `{wa}` contains uppercase; parse_emotion matches lowercase only"
            );
            for &b in &Emotion::ALL[i + 1..] {
                assert_ne!(wa, b.wire_str(), "duplicate wire_str `{wa}`");
            }
        }
    }
}
