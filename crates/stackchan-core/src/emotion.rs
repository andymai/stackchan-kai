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
        }
    }
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
    }
}
