//! [`Director`] — orchestrator that ticks an [`Entity`] each frame.
//!
//! Per frame, the Director:
//!
//! 1. Stamps `entity.tick` (now, `dt_ms`, frame counter).
//! 2. Clears `entity.events` (one-frame fire flags).
//! 3. Runs each registered modifier in `(phase, priority,
//!    registration_order)` order.
//! 4. Polls each registered skill's `should_fire` and invokes the
//!    matching ones.
//!
//! Modifiers are per-frame face / motor / affect mutators living in
//! declared [`Phase`]s; skills are longer-running capabilities with
//! `should_fire` + `invoke`. Skills don't write `face` or `motor`
//! directly (see [`crate::skill`]).
//!
//! Modifiers and skills are held as `&'a mut dyn Modifier` / `&'a mut
//! dyn Skill` references in fixed-capacity [`heapless::Vec`]s. The
//! caller (firmware `render_task` or sim) owns the instances as
//! locals; the Director only borrows. The crate stays `no_std` and
//! alloc-free.
//!
//! [`Entity`]: crate::entity::Entity
//! [`Modifier`]: crate::modifier::Modifier
//! [`Skill`]: crate::skill::Skill

use heapless::Vec;

use crate::clock::Instant;
use crate::entity::{Entity, Tick};
use crate::events::Events;
use crate::modifier::Modifier;
use crate::skill::{Skill, SkillStatus};

/// Maximum number of modifiers a [`Director`] can hold.
pub const MODIFIER_CAP: usize = 32;

/// Maximum number of skills a [`Director`] can hold.
pub const SKILL_CAP: usize = 16;

/// Phases of the per-frame tick.
///
/// `#[repr(u8)]` with numeric gaps of 10 leaves room to insert phases
/// between existing variants without renumbering. Modifiers are sorted
/// by `(phase, priority, registration_order)` before execution.
///
/// Sensors observe the world, cognition picks an intent, emotion
/// follows, speech queues, expression renders, motion executes, audio
/// drives the visual envelope, and output ships the frame.
///
/// Population:
/// - `Affect` (7): `EmotionTouch`, `RemoteCommand`, `PickupReaction`,
///   `WakeOnVoice`, `AmbientSleepy`, `LowBatteryEmotion`, `EmotionCycle`
/// - `Expression` (4): `EmotionStyle`, Blink, Breath, `IdleDrift`
/// - `Motion` (3): `IdleSway`, `EmotionHead`, `ListenHead`
/// - `Audio` (1): `MouthOpenAudio`
/// - `Perception` / `Cognition` / `Speech` / `Output`: empty
///
/// Skills (1): `LookAtSound` (sets `mind.attention = Listening` on
/// sustained `perception.audio_rms`).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Sensors → world model. Empty: the firmware drains Signal
    /// channels into `entity.perception` before `Director::run`.
    Perception = 10,
    /// World model + memory → intent. Empty.
    Cognition = 20,
    /// Intent + sensors → emotion. Touch / Remote / Pickup / Voice
    /// run first (input-edge driven); Ambient / `LowBattery`
    /// (environmental overrides) next; `EmotionCycle` (autonomous
    /// advance) last.
    Affect = 30,
    /// Intent → speech queue. Empty.
    Speech = 40,
    /// Emotion → face style. `EmotionStyle` picks curve / scale /
    /// blush; Blink / Breath / `IdleDrift` add per-frame deltas.
    Expression = 50,
    /// Intent + emotion → pose. `IdleSway` writes a baseline;
    /// `EmotionHead` adds an emotion-keyed bias on top.
    Motion = 60,
    /// Audio-driven visual updates. `MouthOpenAudio` drives
    /// `mouth.mouth_open` from the mic RMS.
    Audio = 70,
    /// Face → frame, pose → servos. Empty for modifiers; the render
    /// task does the draw + servo command after `Director::run`.
    Output = 80,
}

/// Coarse buckets for the [`Field`] enum. Used for human-readable
/// conflict reports and (future) introspection. `Field::group()` maps
/// each fine-grained variant to its bucket.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FieldGroup {
    /// Visual surface (face / style fields).
    Face,
    /// Physical motion (head pose, future arms).
    Motor,
    /// Sensor inputs.
    Perception,
    /// Cognitive layer (affect, autonomy, intent, attention, memory).
    Mind,
    /// Speech I/O.
    Voice,
    /// Pending firmware → modifier inputs (`entity.input.*`).
    Input,
}

/// Fine-grained identifiers for the entity's mutable surface.
///
/// Modifiers declare their `reads` / `writes` via `&'static [Field]`
/// slices on [`ModifierMeta`]. Today the slices are documentation;
/// debug-mode enforcement after each `update` is planned. Per-leaf
/// granularity (e.g. `LeftEyePhase` vs `LeftEyeWeight`) keeps
/// sub-fields of the same component from false-flagging as conflicts.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Field {
    // ---- Face ----
    /// `entity.face.left_eye.phase`
    LeftEyePhase,
    /// `entity.face.left_eye.weight`
    LeftEyeWeight,
    /// `entity.face.left_eye.open_weight`
    LeftEyeOpenWeight,
    /// `entity.face.left_eye.center`
    LeftEyeCenter,
    /// `entity.face.right_eye.phase`
    RightEyePhase,
    /// `entity.face.right_eye.weight`
    RightEyeWeight,
    /// `entity.face.right_eye.open_weight`
    RightEyeOpenWeight,
    /// `entity.face.right_eye.center`
    RightEyeCenter,
    /// `entity.face.mouth.weight`
    MouthWeight,
    /// `entity.face.mouth.mouth_open`
    MouthOpen,
    /// `entity.face.mouth.center`
    MouthCenter,
    /// `entity.face.style.eye_curve`
    EyeCurve,
    /// `entity.face.style.mouth_curve`
    MouthCurve,
    /// `entity.face.style.cheek_blush`
    CheekBlush,
    /// `entity.face.style.eye_scale`
    EyeScale,
    /// `entity.face.style.blink_rate_scale`
    BlinkRateScale,
    /// `entity.face.style.breath_depth_scale`
    BreathDepthScale,

    // ---- Motor ----
    /// `entity.motor.head_pose`
    HeadPose,
    /// `entity.motor.head_pose_actual`
    HeadPoseActual,

    // ---- Perception ----
    /// `entity.perception.accel_g`
    AccelG,
    /// `entity.perception.gyro_dps`
    GyroDps,
    /// `entity.perception.ambient_lux`
    AmbientLux,
    /// `entity.perception.battery_percent`
    BatteryPercent,
    /// `entity.perception.usb_power_present`
    UsbPowerPresent,
    /// `entity.perception.audio_rms`
    AudioRms,

    // ---- Mind ----
    /// `entity.mind.affect.emotion`
    Emotion,
    /// `entity.mind.autonomy.manual_until` + `source`
    Autonomy,
    /// `entity.mind.intent`
    Intent,
    /// `entity.mind.attention`
    Attention,

    // ---- Voice ----
    /// `entity.voice.chirp_request`
    ChirpRequest,

    // ---- Input ----
    /// `entity.input.tap_pending`
    TapPending,
    /// `entity.input.remote_pending`
    RemotePending,
}

impl Field {
    /// Coarse grouping for human-readable reports.
    #[must_use]
    pub const fn group(self) -> FieldGroup {
        match self {
            Self::LeftEyePhase
            | Self::LeftEyeWeight
            | Self::LeftEyeOpenWeight
            | Self::LeftEyeCenter
            | Self::RightEyePhase
            | Self::RightEyeWeight
            | Self::RightEyeOpenWeight
            | Self::RightEyeCenter
            | Self::MouthWeight
            | Self::MouthOpen
            | Self::MouthCenter
            | Self::EyeCurve
            | Self::MouthCurve
            | Self::CheekBlush
            | Self::EyeScale
            | Self::BlinkRateScale
            | Self::BreathDepthScale => FieldGroup::Face,
            Self::HeadPose | Self::HeadPoseActual => FieldGroup::Motor,
            Self::AccelG
            | Self::GyroDps
            | Self::AmbientLux
            | Self::BatteryPercent
            | Self::UsbPowerPresent
            | Self::AudioRms => FieldGroup::Perception,
            Self::Emotion | Self::Autonomy | Self::Intent | Self::Attention => FieldGroup::Mind,
            Self::ChirpRequest => FieldGroup::Voice,
            Self::TapPending | Self::RemotePending => FieldGroup::Input,
        }
    }
}

/// Static metadata for a [`Modifier`] type. Construct as a
/// `const META: ModifierMeta = ...` in each modifier's impl.
#[derive(Debug)]
pub struct ModifierMeta {
    /// Stable identifier — typically the impl's type name.
    pub name: &'static str,
    /// One-sentence description of what this modifier does. Read by
    /// humans and (eventually) a dispatcher.
    pub description: &'static str,
    /// Phase this modifier runs in. Determines coarse ordering.
    pub phase: Phase,
    /// Intra-phase tiebreaker. Lower priority runs first; default `0`.
    pub priority: i8,
    /// Fields this modifier reads.
    pub reads: &'static [Field],
    /// Fields this modifier writes.
    pub writes: &'static [Field],
}

/// Static metadata for a [`Skill`] type: a stable `name` plus a
/// `description` consumable by a dispatcher.
#[derive(Debug)]
pub struct SkillMeta {
    /// Stable identifier.
    pub name: &'static str,
    /// Trigger guidance + action summary.
    pub description: &'static str,
    /// Arbitration priority among overlapping skills. Higher wins.
    pub priority: u8,
    /// Fields this skill is allowed to write. By convention, skills
    /// touch `Mind` / `Voice` / `Events` — `Face` and `Motor` are
    /// modifier territory.
    pub writes: &'static [Field],
}

/// The orchestrator. Holds borrowed modifier + skill registries; ticks
/// the entity each frame.
/// One entry in the modifier registry. Pairs the modifier reference
/// with a registration counter; needed because `core` only provides
/// `sort_unstable_by_key`, so stability on `(phase, priority)` ties
/// requires an explicit secondary key.
struct ModifierSlot<'a> {
    /// 0-based registration order, assigned by `add_modifier`.
    registered_at: u16,
    /// The modifier itself.
    modifier: &'a mut dyn Modifier,
}

/// Error returned when a [`Director`] registry is at capacity.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegistryFull {
    /// Modifier registry is full ([`MODIFIER_CAP`] reached). Caller
    /// should drop one before adding another, or raise the cap and
    /// rebuild.
    Modifiers,
    /// Skill registry is full ([`SKILL_CAP`] reached).
    Skills,
}

impl core::fmt::Display for RegistryFull {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Modifiers => write!(f, "Director modifier registry full ({MODIFIER_CAP})"),
            Self::Skills => write!(f, "Director skill registry full ({SKILL_CAP})"),
        }
    }
}

/// The orchestrator that ticks an [`Entity`] each frame.
///
/// `'a` is the lifetime of the modifier / skill instances; the caller
/// owns them as locals and registers `&'a mut dyn` references.
pub struct Director<'a> {
    /// Registered modifiers, sorted by `(phase, priority,
    /// registered_at)` on first `run()` call.
    modifiers: Vec<ModifierSlot<'a>, MODIFIER_CAP>,
    /// Registered skills.
    skills: Vec<&'a mut dyn Skill, SKILL_CAP>,
    /// Whether `modifiers` is sorted. Cleared on registration; set
    /// by the first `run()` that observes the registry.
    sorted: bool,
    /// Monotonic frame counter, written into `entity.tick.frame`.
    frame: u64,
    /// `Some` after the first `run()`; used to compute `dt_ms`.
    last_now: Option<Instant>,
    /// Next registration counter to assign.
    next_registration: u16,
}

impl Default for Director<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Director<'a> {
    /// Construct an empty Director with no modifiers or skills.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modifiers: Vec::new(),
            skills: Vec::new(),
            sorted: false,
            frame: 0,
            last_now: None,
            next_registration: 0,
        }
    }

    /// Register a modifier. Insertion order within the same `(phase,
    /// priority)` is preserved on the sort.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFull::Modifiers`] if the modifier registry is
    /// full ([`MODIFIER_CAP`]).
    pub fn add_modifier(&mut self, m: &'a mut dyn Modifier) -> Result<&mut Self, RegistryFull> {
        let slot = ModifierSlot {
            registered_at: self.next_registration,
            modifier: m,
        };
        self.modifiers
            .push(slot)
            .map_err(|_| RegistryFull::Modifiers)?;
        self.next_registration = self.next_registration.saturating_add(1);
        self.sorted = false;
        Ok(self)
    }

    /// Register a skill.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryFull::Skills`] if the skill registry is full
    /// ([`SKILL_CAP`]).
    pub fn add_skill(&mut self, s: &'a mut dyn Skill) -> Result<&mut Self, RegistryFull> {
        self.skills.push(s).map_err(|_| RegistryFull::Skills)?;
        Ok(self)
    }

    /// Tick the entity one frame:
    /// 1. Sort modifiers by `(phase, priority)` if not already sorted.
    /// 2. Stamp `entity.tick`.
    /// 3. Clear `entity.events` (one-frame fire flags).
    /// 4. Iterate modifiers, calling `update`.
    /// 5. Iterate skills, calling `invoke` on those whose `should_fire`
    ///    returns true.
    pub fn run(&mut self, entity: &mut Entity, now: Instant) {
        if !self.sorted {
            // `sort_unstable_by_key` is the no_std-friendly sort.
            // Stability comes from including `registered_at` in the
            // key: distinct slots can never produce equal keys, so the
            // result matches `(phase, priority, registration_order)`.
            self.modifiers.as_mut_slice().sort_unstable_by_key(|slot| {
                let meta = slot.modifier.meta();
                (meta.phase, meta.priority, slot.registered_at)
            });
            // Skills sort by priority descending (higher fires first),
            // opposite of modifiers.
            self.skills
                .as_mut_slice()
                .sort_unstable_by_key(|s| core::cmp::Reverse(s.meta().priority));
            self.sorted = true;
        }

        // Frame bookkeeping.
        self.frame = self.frame.saturating_add(1);
        let dt_ms = self.last_now.map_or(0, |prev| {
            u32::try_from(now.as_millis().saturating_sub(prev.as_millis())).unwrap_or(u32::MAX)
        });
        entity.tick = Tick {
            now,
            dt_ms,
            frame: self.frame,
        };

        // Clear one-frame fire flags. Modifiers populate events during
        // their pass; firmware reads them after `run()` returns.
        entity.events = Events::default();

        for slot in &mut self.modifiers {
            slot.modifier.update(entity);
        }

        for s in &mut self.skills {
            if s.should_fire(entity) {
                // `should_fire` is the only gate; `SkillStatus::Done`
                // vs `Continuing` is reserved for skill state-tracking.
                let _status: SkillStatus = s.invoke(entity);
            }
        }

        self.last_now = Some(now);
    }

    /// Number of registered modifiers.
    #[must_use]
    pub fn modifier_count(&self) -> usize {
        self.modifiers.len()
    }

    /// Number of registered skills.
    #[must_use]
    pub fn skill_count(&self) -> usize {
        self.skills.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    // Test fixture: a modifier that records the order it was called.
    struct OrderRecorder {
        meta: &'static ModifierMeta,
        // Indirection so we can construct multiple instances with
        // distinct meta. Real modifiers use `Self::META`.
        log_value: u8,
    }

    static M_AFFECT_HIGH: ModifierMeta = ModifierMeta {
        name: "AffectHighPriority",
        description: "test fixture",
        phase: Phase::Affect,
        priority: -10,
        reads: &[],
        writes: &[],
    };
    static M_AFFECT_LOW: ModifierMeta = ModifierMeta {
        name: "AffectLowPriority",
        description: "test fixture",
        phase: Phase::Affect,
        priority: 10,
        reads: &[],
        writes: &[],
    };
    static M_EXPRESSION: ModifierMeta = ModifierMeta {
        name: "Expression",
        description: "test fixture",
        phase: Phase::Expression,
        priority: 0,
        reads: &[],
        writes: &[],
    };

    impl Modifier for OrderRecorder {
        fn meta(&self) -> &'static ModifierMeta {
            self.meta
        }
        fn update(&mut self, entity: &mut Entity) {
            // Encode call order: shift the prior value into a higher
            // place and append `log_value`. Saturating math so we never
            // overflow regardless of how many values get appended.
            let prev = u32::from(entity.face.left_eye.weight);
            let next = prev
                .saturating_mul(10)
                .saturating_add(u32::from(self.log_value));
            entity.face.left_eye.weight = u8::try_from(next.min(255)).unwrap_or(255);
        }
    }

    #[test]
    fn modifier_phase_order_preserved() {
        let mut affect_low = OrderRecorder {
            meta: &M_AFFECT_LOW,
            log_value: 2,
        };
        let mut affect_high = OrderRecorder {
            meta: &M_AFFECT_HIGH,
            log_value: 1,
        };
        let mut expr = OrderRecorder {
            meta: &M_EXPRESSION,
            log_value: 3,
        };

        let mut director = Director::new();
        // Register out of order on purpose; sort should fix it.
        director.add_modifier(&mut expr).unwrap();
        director.add_modifier(&mut affect_low).unwrap();
        director.add_modifier(&mut affect_high).unwrap();

        let mut entity = Entity::default();
        // Zero out the test slot so the recorder starts fresh.
        entity.face.left_eye.weight = 0;
        director.run(&mut entity, Instant::from_millis(0));

        // Expected execution order:
        //   AffectHigh (priority -10) → AffectLow (10) → Expression
        // log_value writes: 1, 2, 3 → eye.weight = ((0*10+1)*10+2)*10+3 = 123
        assert_eq!(
            entity.face.left_eye.weight, 123,
            "modifiers ran in wrong order"
        );
    }

    #[test]
    fn events_cleared_at_frame_start() {
        // `Events` is empty today; the lifecycle contract — that
        // `Director::run` reassigns the struct to `Default` — is the
        // load-bearing invariant. This test pins it so a future
        // re-introduced field doesn't silently lose its frame-start
        // clear.
        let mut director: Director = Director::new();
        let mut entity = Entity::default();
        let before = entity.events;
        director.run(&mut entity, Instant::from_millis(0));
        assert_eq!(entity.events, before, "events struct was reset to default");
    }

    #[test]
    fn tick_stamped_each_frame() {
        let mut director: Director = Director::new();
        let mut entity = Entity::default();

        director.run(&mut entity, Instant::from_millis(100));
        assert_eq!(entity.tick.frame, 1);
        assert_eq!(entity.tick.now.as_millis(), 100);
        assert_eq!(entity.tick.dt_ms, 0); // first frame, no prev

        director.run(&mut entity, Instant::from_millis(133));
        assert_eq!(entity.tick.frame, 2);
        assert_eq!(entity.tick.now.as_millis(), 133);
        assert_eq!(entity.tick.dt_ms, 33);
    }

    #[test]
    fn field_group_buckets_correctly() {
        assert_eq!(Field::LeftEyePhase.group(), FieldGroup::Face);
        assert_eq!(Field::HeadPose.group(), FieldGroup::Motor);
        assert_eq!(Field::AmbientLux.group(), FieldGroup::Perception);
        assert_eq!(Field::Emotion.group(), FieldGroup::Mind);
        assert_eq!(Field::ChirpRequest.group(), FieldGroup::Voice);
        assert_eq!(Field::TapPending.group(), FieldGroup::Input);
    }
}
