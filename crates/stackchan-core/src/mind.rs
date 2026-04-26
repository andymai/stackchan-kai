//! Cognitive layer of the entity.
//!
//! [`Mind`] holds [`Affect`] (current emotion), [`Autonomy`]
//! (manual-override gating), [`Intent`] (current goal), and
//! [`Attention`] (current focus). [`Memory`] is a marker type
//! reserved for future cross-boot persistence. Modifiers in
//! [`Phase::Affect`] write `mind.affect.emotion` and
//! `mind.autonomy.manual_until`; skills write `mind.intent` and
//! `mind.attention`; modifiers in [`Phase::Expression`] and
//! [`Phase::Motion`] read `mind.affect.emotion` (for style + pose
//! bias) and `mind.attention` (for attention-driven pose).
//!
//! Splitting `Affect` (what the entity feels) from `Autonomy` (whether
//! autonomous drivers are allowed to override) lets emotion drivers
//! always propose a value; `Autonomy::manual_until` decides whether
//! the proposal is accepted. The [`Autonomy::source`] field lets
//! priority-based override management distinguish a touch hold from a
//! low-battery hold.
//!
//! [`Phase::Affect`]: crate::director::Phase::Affect
//! [`Phase::Expression`]: crate::director::Phase::Expression
//! [`Phase::Motion`]: crate::director::Phase::Motion

use crate::clock::Instant;
use crate::emotion::Emotion;

/// The entity's current felt state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Affect {
    /// Current emotional expression. Set by `Phase::Affect` modifiers
    /// (touch, IR, voice, ambient, battery, autonomous cycler);
    /// consumed by `EmotionStyle` and `EmotionHead`.
    pub emotion: Emotion,
}

/// Where an autonomy override originated. Lets layered override
/// management distinguish hold sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverrideSource {
    /// Front-screen tap (FT6336U).
    Touch,
    /// Back-of-head body-touch tap (`Si12T` 3-zone pads).
    BodyTouch,
    /// IR-remote button press.
    Remote,
    /// IMU pickup detection — set by `IntentReflex` on a transition
    /// into [`Intent::PickedUp`].
    Pickup,
    /// IMU shake detection — set by `IntentReflex` on a transition
    /// into [`Intent::Shaken`].
    Shake,
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
/// `manual_until = Some(t)` means autonomous drivers stand down until
/// the clock reaches `t`. `EmotionCycle` checks this before advancing;
/// touch / remote / pickup / voice / environmental overrides set this
/// when they take control.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Autonomy {
    /// Deadline until which autonomous emotion drivers should defer.
    /// `None` = autonomy active.
    pub manual_until: Option<Instant>,
    /// Which kind of override is holding the gate. `None` when
    /// `manual_until` is `None`.
    pub source: Option<OverrideSource>,
}

/// Current goal or planned action of the entity. Set by skills and
/// read by modifiers in later phases. Default: [`Intent::Idle`].
///
/// ## Priority
///
/// Multiple skills may try to write `intent` on the same tick. The
/// [`crate::skills::Handling`] skill resolves IMU-derived states
/// against [`crate::skills::Petting`] using the order
/// `PickedUp > Shaken > BeingPet > Tilted > Listen > Idle` — physical
/// handling of the whole avatar dominates over local body-touch, and
/// passive pose (`Tilted`) is lowest because it's not active handling.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Intent {
    /// No active goal.
    #[default]
    Idle,
    /// Listening to ambient sound. Set by
    /// [`crate::skills::LookAtSound`]; cleared on release.
    Listen,
    /// Being pet on the back-of-head strip. Set by
    /// [`crate::skills::Petting`] after sustained any-zone contact;
    /// cleared on release.
    BeingPet,
    /// Held in the air. Set by [`crate::skills::Handling`] after
    /// sustained `|accel| ≠ 1 g`. Cleared when the avatar settles.
    /// `IntentReflex` translates the entry edge to `Surprised`.
    PickedUp,
    /// Being shaken. Set by [`crate::skills::Handling`] when accel
    /// oscillates above the shake threshold within a short window.
    /// `IntentReflex` translates the entry edge to `Angry`.
    Shaken,
    /// Lying on its side / face-down. Set by [`crate::skills::Handling`]
    /// when the gravity vector deviates from face-up for a sustained
    /// window. Passive — no reflex emotion attached; downstream
    /// modifiers (e.g. an extended `IntentStyle`) own the visual.
    Tilted,
}

/// What the entity is currently focused on.
///
/// Carries enough state for downstream modifiers to animate the focus
/// (e.g. ease-in based on `since`). Default: [`Attention::None`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Attention {
    /// Not focused on anything specific.
    #[default]
    None,
    /// Listening to a sound source. `since` is the instant the
    /// listening attention began; consumers (e.g.
    /// [`crate::modifiers::ListenHead`]) use it for ease-in animation.
    Listening {
        /// When the listening attention began.
        since: Instant,
    },
}

/// Persistent facts the entity remembers across boots. Placeholder
/// marker type; not yet populated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Memory;

/// The cognitive layer of the entity. Sub-component shape is stable;
/// new fields land on `Affect` / `Autonomy` / `Intent` / `Attention` /
/// `Memory` without breaking modifiers that read `Mind`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mind {
    /// Current felt state.
    pub affect: Affect,
    /// Manual-override gating.
    pub autonomy: Autonomy,
    /// Current goal or planned action.
    pub intent: Intent,
    /// What the entity is focused on.
    pub attention: Attention,
    /// Persistent facts.
    pub memory: Memory,
}
