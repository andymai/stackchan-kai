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
use crate::head::Pose;

/// The entity's current felt state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Affect {
    /// Current emotional expression. Set by `Phase::Affect` modifiers
    /// (touch, IR, voice, ambient, battery, autonomous cycler);
    /// consumed by `StyleFromEmotion` and `HeadFromEmotion`.
    pub emotion: Emotion,
}

/// Where an autonomy override originated. Lets layered override
/// management distinguish hold sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverrideSource {
    /// Front-screen (avatar's face) tap (FT6336U).
    FaceTouch,
    /// Back-of-head body-touch tap (`Si12T` 3-zone pads).
    BodyTouch,
    /// IR-remote button press.
    Remote,
    /// IMU pickup detection — set by `EmotionFromIntent` on a transition
    /// into [`Intent::PickedUp`].
    Pickup,
    /// IMU shake detection — set by `EmotionFromIntent` on a transition
    /// into [`Intent::Shaken`].
    Shake,
    /// Sustained voice activity.
    Voice,
    /// High-level acoustic transient — clap, shout, slam — set by
    /// `IntentFromLoud` when `audio_rms` crosses the loud threshold
    /// on a single tick.
    Startle,
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
/// `Startled > PickedUp > Shaken > Petted > Tilted > Listening > Idle`
/// — a startle-class transient outranks even physical handling
/// (the avatar reacts to a loud noise even mid-pickup), passive
/// pose (`Tilted`) is lowest because it's not active handling.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Intent {
    /// No active goal.
    #[default]
    Idle,
    /// Listening to ambient sound. Set by
    /// [`crate::skills::Listening`]; cleared on release.
    Listening,
    /// Reacting to an acoustic transient (clap, shout, slam). Set by
    /// [`crate::modifiers::IntentFromLoud`] on the rising edge across
    /// the loud-threshold; cleared by the modifier when the hold
    /// expires. `EmotionFromIntent` does not own the emotion for this
    /// intent — `IntentFromLoud` writes `Surprised` directly to keep
    /// reaction latency to a single tick.
    Startled,
    /// Being pet on the back-of-head strip. Set by
    /// [`crate::skills::Petting`] after sustained any-zone contact;
    /// cleared on release.
    Petted,
    /// Held in the air. Set by [`crate::skills::Handling`] after
    /// sustained `|accel| ≠ 1 g`. Cleared when the avatar settles.
    /// `EmotionFromIntent` translates the entry edge to `Surprised`.
    PickedUp,
    /// Being shaken. Set by [`crate::skills::Handling`] when accel
    /// oscillates above the shake threshold within a short window.
    /// `EmotionFromIntent` translates the entry edge to `Angry`.
    Shaken,
    /// Lying on its side / face-down. Set by [`crate::skills::Handling`]
    /// when the gravity vector deviates from face-up for a sustained
    /// window. Passive — no reflex emotion attached; downstream
    /// modifiers (e.g. an extended `StyleFromIntent`) own the visual.
    Tilted,
}

/// What the entity is currently focused on.
///
/// Carries enough state for downstream modifiers to animate the focus
/// (e.g. ease-in based on `since`). Default: [`Attention::None`].
///
/// Only `PartialEq` (not `Eq`) because [`Attention::Tracking`] carries
/// a [`Pose`] with f32 fields. Modifier tests still use `assert_eq!`
/// freely — that needs only `PartialEq`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Attention {
    /// Not focused on anything specific.
    #[default]
    None,
    /// Listening to a sound source. `since` is the instant the
    /// listening attention began; consumers (e.g.
    /// [`crate::modifiers::HeadFromAttention`]) use it for ease-in animation.
    Listening {
        /// When the listening attention began.
        since: Instant,
    },
    /// Tracking a moving target detected by the camera. `target` is
    /// the head-pose the engine wants to look at (already in safe
    /// pan/tilt range — the firmware tracker handles clamping); `since`
    /// is the instant tracking attention began. Set by
    /// [`crate::modifiers::AttentionFromTracking`].
    Tracking {
        /// Where the head should look. In the same coordinate system
        /// as `motor.head_pose`.
        target: Pose,
        /// When the tracking attention began.
        since: Instant,
    },
}

/// Face-lock engagement state.
///
/// Orthogonal to [`Attention`]: motion-tracking attention attracts
/// the head toward whatever moved; engagement records whether the
/// firmware-side face cascade has confirmed a face inside that motion
/// region, with hysteresis so the head doesn't twitch on single-frame
/// detector flickers.
///
/// Lifecycle:
/// `Idle → Locking { hits } → Locked → Releasing { misses } → Idle`.
/// Hit / miss thresholds are configurable on
/// [`crate::modifiers::AttentionFromTracking`] (default: 3 frames to
/// engage, 10 frames to release). Set by
/// [`crate::modifiers::AttentionFromTracking`]; consumed by engagement
/// modifiers (`HeadLagFromGaze`, `Blink`, lost-target search).
///
/// `centroid` is in normalised frame coordinates `[-1, 1]` — same
/// convention as [`crate::perception::TrackingObservation::face_centroid`].
/// `at` is the wall-clock time the centroid was last refreshed; the
/// lost-target search choreography in [`Engagement::Releasing`] uses
/// it to time the "hold last pose then look around" beat.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Engagement {
    /// No face seen recently. Default state.
    #[default]
    Idle,
    /// Face detected for `hits` consecutive frames; not yet locked.
    /// Resets to [`Engagement::Idle`] on the first miss before the
    /// lock threshold.
    Locking {
        /// Consecutive frames the cascade has fired in this run.
        hits: u8,
    },
    /// Lock engaged. The head and eyes should now follow `centroid`
    /// preferentially (eye-leads-head, blink-rate boost, etc.).
    Locked {
        /// Most recently observed face centroid in `[-1, 1]` per axis.
        centroid: (f32, f32),
        /// Wall-clock time `centroid` was last refreshed.
        at: Instant,
    },
    /// Lock fading. The face was lost `misses` frames ago — modifiers
    /// continue acting on `centroid` for the brief search beat, then
    /// transition to [`Engagement::Idle`] when `misses` exceeds the
    /// release threshold.
    Releasing {
        /// Last known face centroid before the lock was lost.
        centroid: (f32, f32),
        /// Wall-clock time the lock was last fresh
        /// (= last `Engagement::Locked.at`).
        at: Instant,
        /// Consecutive frames without a face since `at`.
        misses: u8,
    },
}

impl Engagement {
    /// `true` iff the engagement state is currently driving behaviour
    /// (i.e. eyes/head should privilege the face centroid). Both
    /// [`Engagement::Locked`] and [`Engagement::Releasing`] qualify;
    /// `Locking` does not, because the lock hasn't crossed the
    /// hysteresis threshold yet.
    #[must_use]
    pub const fn is_engaged(&self) -> bool {
        matches!(self, Self::Locked { .. } | Self::Releasing { .. })
    }

    /// The most recently observed face centroid, if the engagement
    /// state carries one. `None` for [`Engagement::Idle`] /
    /// [`Engagement::Locking`].
    #[must_use]
    pub const fn centroid(&self) -> Option<(f32, f32)> {
        match self {
            Self::Locked { centroid, .. } | Self::Releasing { centroid, .. } => Some(*centroid),
            Self::Idle | Self::Locking { .. } => None,
        }
    }
}

/// Persistent facts the entity remembers across boots. Placeholder
/// marker type; not yet populated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Memory;

/// The cognitive layer of the entity. Sub-component shape is stable;
/// new fields land on `Affect` / `Autonomy` / `Intent` / `Attention` /
/// `Memory` without breaking modifiers that read `Mind`.
///
/// `PartialEq` only — [`Attention::Tracking`] holds a [`Pose`] with
/// f32 fields, which leak through here.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Mind {
    /// Current felt state.
    pub affect: Affect,
    /// Manual-override gating.
    pub autonomy: Autonomy,
    /// Current goal or planned action.
    pub intent: Intent,
    /// What the entity is focused on.
    pub attention: Attention,
    /// Face-lock state riding on top of [`Self::attention`]. Set by
    /// [`crate::modifiers::AttentionFromTracking`] from the firmware-side
    /// cascade observations; read by engagement modifiers.
    pub engagement: Engagement,
    /// Persistent facts.
    pub memory: Memory,
}
