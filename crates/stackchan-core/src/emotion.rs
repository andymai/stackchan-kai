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
}
