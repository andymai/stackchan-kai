//! Cognitive layer of the entity.
//!
//! [`Mind`] holds [`Affect`] (current emotion) and [`Autonomy`]
//! (manual-override gating). [`Intent`], [`Attention`], and [`Memory`]
//! are placeholder marker types reserved for future use. Modifiers in
//! [`Phase::Affect`] write `mind.affect.emotion` and
//! `mind.autonomy.manual_until`; modifiers in [`Phase::Expression`]
//! and [`Phase::Motion`] read `mind.affect.emotion` to choose a face
//! style and head bias.
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

/// Current goal or planned action of the entity. Placeholder marker
/// type; not yet populated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Intent;

/// What the entity is currently focused on. Placeholder marker type;
/// not yet populated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Attention;

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
