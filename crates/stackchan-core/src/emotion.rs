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
