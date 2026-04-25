//! Cognitive layer of the entity: emotion, autonomy, intent, attention,
//! memory.
//!
//! [`Mind`] is the brain. Today it carries [`Affect`] (current emotion)
//! and [`Autonomy`] (manual-override gating); [`Intent`], [`Attention`],
//! and [`Memory`] are stub marker types reserved for v2.x. Modifiers in
//! [`Phase::Affect`] write to `mind.affect.emotion` and
//! `mind.autonomy.manual_until`; modifiers in [`Phase::Expression`] and
//! [`Phase::Motion`] read `mind.affect.emotion` to choose a face style
//! and head-bias; future skills will write to `mind.intent` to drive
//! both expression and speech.
//!
//! ## Design rationale
//!
//! The split between `Affect` (what the entity *feels*) and `Autonomy`
//! (whether autonomous drivers are allowed to override) is deliberate.
//! In v0.x both lived flat on `Avatar`, which conflated "current
//! emotion" with "manual override is active." The cleaner shape: an
//! emotion driver always proposes; `Autonomy::manual_until` decides
//! whether the proposal is accepted. The new
//! [`Autonomy::source`] field will let v2.x skills do layered overrides
//! (a "conversation" skill can override `Source::LowBattery` but not
//! `Source::Pickup`) without touching the autonomy gate's expiry.
//!
//! [`Phase::Affect`]: crate::director::Phase::Affect
//! [`Phase::Expression`]: crate::director::Phase::Expression
//! [`Phase::Motion`]: crate::director::Phase::Motion

use crate::clock::Instant;
use crate::emotion::Emotion;

/// The entity's current felt state.
///
/// Today: the discrete [`Emotion`] enum from the v0.x model. v2.x will
/// extend this with continuous valence/arousal axes for richer
/// dimensional affect models without breaking modifiers that read
/// [`Self::emotion`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Affect {
    /// Current emotional expression. Set by `Phase::Affect` modifiers
    /// (touch input, IR remote, voice activity, ambient light, battery
    /// state, autonomous cycler); consumed by `EmotionStyle` and
    /// `EmotionHead` to translate into face/style and head-bias.
    pub emotion: Emotion,
    // v2.x: pub valence: f32, pub arousal: f32,
}

/// Where an autonomy override originated. Lets future skills do
/// priority-based override management ("conversation skill may
/// override `LowBattery` but not `Pickup`").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverrideSource {
    /// Touch tap from the user.
    Touch,
    /// IR-remote button press.
    Remote,
    /// IMU pickup detection.
    Pickup,
    /// Sustained voice activity.
    Voice,
    /// Ambient-light-driven sleepy override.
    Ambient,
    /// Low-battery sleepy override.
    LowBattery,
}

/// Autonomy gating: lets an explicit driver pin emotion against the
/// autonomous cycler.
///
/// `manual_until = Some(t)` means "autonomous drivers stand down until
/// the clock reaches `t`." `EmotionCycle` checks this before
/// advancing; touch / remote / pickup / voice / environmental
/// overrides all set this when they take control.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Autonomy {
    /// Deadline until which autonomous emotion drivers should defer.
    /// `None` = autonomy active (default).
    pub manual_until: Option<Instant>,
    /// Which kind of override is currently holding the gate. `None`
    /// when `manual_until` is `None`. v2.x: skills consult this for
    /// layered override management.
    pub source: Option<OverrideSource>,
}

/// Current goal / planned action of the entity.
///
/// **Stub for v2.x.** Today this is empty; future shape:
/// ```ignore
/// pub struct Intent {
///     pub kind: IntentKind,        // Idle, Greeting, Listening, Speaking, Reacting
///     pub started_at: Option<Instant>,
///     pub target: Option<AttentionFocus>,
/// }
/// ```
/// The `MindBridge` async firmware task (LAN HTTP/MQTT to the LLM
/// host) will publish results into this field; modifiers in
/// [`Phase::Speech`] / [`Phase::Motion`] will read it to drive
/// behaviour.
///
/// [`Phase::Speech`]: crate::director::Phase::Speech
/// [`Phase::Motion`]: crate::director::Phase::Motion
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Intent;

/// What the entity is currently focused on.
///
/// **Stub for v2.x.** Today this is empty; future shape:
/// ```ignore
/// pub struct Attention {
///     pub focus: AttentionFocus,   // None, Toward(Direction), Listening
///     pub since: Option<Instant>,
/// }
/// ```
/// Used by future eye-gaze and head-pointing modifiers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Attention;

/// Persistent facts the entity remembers across boots.
///
/// **Stub for v2.x.** Today this is empty; future shape backs onto
/// ESP-IDF NVS (non-volatile storage) for a typed key-value store:
/// preferences, recent conversation summaries, learned facts. The
/// API will be `entity.mind.memory.get::<T>(key)` /
/// `entity.mind.memory.put(key, value)`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Memory;

/// The cognitive layer of the entity.
///
/// Composed of five sub-components — [`Affect`] and [`Autonomy`] are
/// active today, [`Intent`] / [`Attention`] / [`Memory`] are
/// reserved for v2.x. The container shape is stable; new fields can
/// land on the sub-components without breaking modifiers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mind {
    /// Current felt state.
    pub affect: Affect,
    /// Manual-override gating.
    pub autonomy: Autonomy,
    /// Current goal / planned action. v2.x.
    pub intent: Intent,
    /// What the entity is focused on. v2.x.
    pub attention: Attention,
    /// Persistent facts. v2.x (NVS-backed).
    pub memory: Memory,
}
