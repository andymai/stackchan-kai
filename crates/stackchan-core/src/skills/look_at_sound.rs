//! `LookAtSound`: attention-shifting skill driven by sustained voice
//! activity.
//!
//! ## Trigger shape
//!
//! Watches `entity.perception.audio_rms` (linear, `0.0..=1.0`).
//! Counts consecutive ticks above [`LISTEN_RMS_THRESHOLD`]; once the
//! run length reaches [`LISTEN_SUSTAIN_TICKS`] the skill fires:
//!
//! - `entity.mind.intent` ← [`Intent::Listen`]
//! - `entity.mind.attention` ← [`Attention::Listening { since: now }`]
//!
//! Once fired, attention persists while loudness continues. After the
//! audio drops back below threshold, the skill clears intent +
//! attention back to defaults [`LISTEN_RELEASE_MS`] later.
//!
//! Unknown audio (`audio_rms = None`, before the firmware publishes
//! its first window) is treated as silence and never triggers.
//!
//! ## Relationship to `WakeOnVoice`
//!
//! [`WakeOnVoice`](crate::modifiers::WakeOnVoice) reacts to the same
//! `audio_rms` trigger but writes `mind.affect.emotion = Happy` and
//! queues `voice.chirp_request = Wake`. The two are complementary: a
//! sustained voice burst makes the entity *feel* happy (`WakeOnVoice`)
//! and *focus its attention* on the sound (this skill). They share
//! [`LISTEN_RMS_THRESHOLD`] / [`LISTEN_SUSTAIN_TICKS`] values so both
//! fire on the same tick.
//!
//! ## Why a Skill, not a Modifier
//!
//! The output is *attention*, not face / motor. Skills write
//! `mind.intent` / `mind.attention` / `voice` / `events` by contract;
//! modifiers translate that into face + pose. The companion
//! [`ListenHead`](crate::modifiers::ListenHead) modifier consumes
//! `mind.attention` and biases head tilt accordingly.

use crate::clock::Instant;
use crate::director::{Field, SkillMeta};
use crate::entity::Entity;
use crate::mind::{Attention, Intent};
use crate::modifiers::{WAKE_RMS_THRESHOLD, WAKE_SUSTAIN_TICKS};
use crate::skill::{Skill, SkillStatus};

/// Linear RMS threshold for the "loud" classification.
///
/// Re-exports [`WAKE_RMS_THRESHOLD`] so attention and emotion shifts
/// fire on the same audio event — a single source of truth prevents
/// the two detectors from drifting apart.
pub const LISTEN_RMS_THRESHOLD: f32 = WAKE_RMS_THRESHOLD;

/// Consecutive loud ticks required to enter Listening attention.
///
/// Re-exports [`WAKE_SUSTAIN_TICKS`] for the same reason as
/// [`LISTEN_RMS_THRESHOLD`]: pin both detectors to the same trigger
/// shape so they fire on the same tick.
pub const LISTEN_SUSTAIN_TICKS: u8 = WAKE_SUSTAIN_TICKS;

/// How long Listening attention persists after audio drops back below
/// threshold, in ms.
///
/// 1500 ms reads as "the entity stayed engaged for a moment after the
/// sound stopped" without lingering so long it fights other behaviors.
pub const LISTEN_RELEASE_MS: u64 = 1_500;

/// Attention-shifting skill driven by sustained voice activity. See
/// the module docs for trigger shape.
#[derive(Debug, Clone, Copy)]
pub struct LookAtSound {
    /// Linear-RMS threshold above which a tick counts as loud.
    pub threshold: f32,
    /// Consecutive-loud-ticks required to enter Listening.
    pub sustain_ticks: u8,
    /// Hold window after audio quiets before clearing attention.
    pub release_ms: u64,
    /// Running count of consecutive loud ticks. Reset on any quiet
    /// tick. Saturates at `u8::MAX` so a very long sustained run
    /// doesn't wrap.
    consecutive_loud: u8,
    /// Instant of the most recent loud tick, used to time the
    /// post-quiet release window. `None` between bursts.
    last_loud: Option<Instant>,
}

impl LookAtSound {
    /// Construct with default tuning ([`LISTEN_RMS_THRESHOLD`] /
    /// [`LISTEN_SUSTAIN_TICKS`] / [`LISTEN_RELEASE_MS`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold: LISTEN_RMS_THRESHOLD,
            sustain_ticks: LISTEN_SUSTAIN_TICKS,
            release_ms: LISTEN_RELEASE_MS,
            consecutive_loud: 0,
            last_loud: None,
        }
    }
}

impl Default for LookAtSound {
    fn default() -> Self {
        Self::new()
    }
}

impl Skill for LookAtSound {
    fn meta(&self) -> &'static SkillMeta {
        static META: SkillMeta = SkillMeta {
            name: "LookAtSound",
            description: "Sustained perception.audio_rms above LISTEN_RMS_THRESHOLD sets \
                          mind.intent=Listen and mind.attention=Listening{since}. Attention \
                          clears LISTEN_RELEASE_MS after audio quiets. Pairs with the \
                          ListenHead motion modifier for a cocked-head listening posture.",
            priority: 50,
            writes: &[Field::Intent, Field::Attention],
        };
        &META
    }

    fn should_fire(&self, _entity: &Entity) -> bool {
        true
    }

    fn invoke(&mut self, entity: &mut Entity) -> SkillStatus {
        let now = entity.tick.now;
        let loud = entity
            .perception
            .audio_rms
            .is_some_and(|rms| rms > self.threshold);

        if loud {
            self.consecutive_loud = self.consecutive_loud.saturating_add(1);
            self.last_loud = Some(now);
        } else {
            self.consecutive_loud = 0;
        }

        if self.consecutive_loud >= self.sustain_ticks {
            // Sustained loud: enter or maintain Listening. `since`
            // pins to the first frame of this run — re-entering
            // doesn't reset it mid-sustain.
            if !matches!(entity.mind.attention, Attention::Listening { .. }) {
                entity.mind.attention = Attention::Listening { since: now };
            }
            entity.mind.intent = Intent::Listen;
            return SkillStatus::Continuing;
        }

        if matches!(entity.mind.attention, Attention::Listening { .. }) {
            // Inside the release window of a recent burst → hold.
            if self
                .last_loud
                .is_some_and(|t| now.saturating_duration_since(t) < self.release_ms)
            {
                return SkillStatus::Continuing;
            }
            entity.mind.attention = Attention::None;
            entity.mind.intent = Intent::Idle;
            self.last_loud = None;
        } else if !loud {
            // Not listening, not loud: drop any stale anchor left by
            // a sub-sustain blip so it can't gate a spurious hold on
            // the next entry into Listening.
            self.last_loud = None;
        }
        SkillStatus::Done
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "let-else with panic is the cleanest pattern for value extraction \
              on enum variants in tests"
)]
mod tests {
    use super::*;

    fn loud_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.audio_rms = Some(0.3);
        e
    }

    fn quiet_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.audio_rms = Some(0.001);
        e
    }

    /// Drive the skill's state machine for `ticks` frames at 33 ms /
    /// tick, starting at `start_ms`. Mirrors the cadence of the
    /// firmware render loop.
    fn step(skill: &mut LookAtSound, entity: &mut Entity, start_ms: u64, ticks: u64) {
        for t in 0..ticks {
            entity.tick.now = Instant::from_millis(start_ms + t * 33);
            let _ = skill.invoke(entity);
        }
    }

    #[test]
    fn never_fires_on_silence() {
        let mut skill = LookAtSound::new();
        let mut entity = quiet_avatar();
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) + 5,
        );
        assert_eq!(entity.mind.attention, Attention::None);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn unknown_audio_is_silence() {
        // `audio_rms = None` (default) before the audio task
        // publishes. Treated as silent.
        let mut skill = LookAtSound::new();
        let mut entity = Entity::default();
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) + 5,
        );
        assert_eq!(entity.mind.attention, Attention::None);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn sustained_loud_enters_listening() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(&mut skill, &mut entity, 0, u64::from(LISTEN_SUSTAIN_TICKS));
        assert!(matches!(entity.mind.attention, Attention::Listening { .. }));
        assert_eq!(entity.mind.intent, Intent::Listen);
    }

    #[test]
    fn loud_below_sustain_does_not_enter() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) - 1,
        );
        assert_eq!(entity.mind.attention, Attention::None);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn quiet_tick_resets_counter_mid_burst() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        // SUSTAIN_TICKS - 1 loud, then one quiet (resets), then a
        // few more loud — should not trigger because each run is
        // below sustain.
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) - 1,
        );
        entity.perception.audio_rms = Some(0.001);
        step(&mut skill, &mut entity, 1_000, 1);
        entity.perception.audio_rms = Some(0.3);
        step(
            &mut skill,
            &mut entity,
            2_000,
            u64::from(LISTEN_SUSTAIN_TICKS) - 1,
        );
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn since_timestamp_pins_to_first_listening_frame() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        // Drive past the sustain threshold.
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) + 5,
        );
        let Attention::Listening { since: first } = entity.mind.attention else {
            panic!("expected Listening, got {:?}", entity.mind.attention);
        };
        // Continue loud for several more ticks. `since` must stay
        // pinned to the first listening frame — consumers depend on
        // this to compute ease-in elapsed time.
        step(
            &mut skill,
            &mut entity,
            10_000,
            u64::from(LISTEN_SUSTAIN_TICKS),
        );
        let Attention::Listening { since: later } = entity.mind.attention else {
            panic!("expected Listening still, got {:?}", entity.mind.attention);
        };
        assert_eq!(first, later, "since must not advance during a sustain run");
    }

    #[test]
    fn quiet_within_release_window_holds_attention() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(&mut skill, &mut entity, 0, u64::from(LISTEN_SUSTAIN_TICKS));
        let last_loud_at = entity.tick.now;
        // Go quiet but stay inside the release window (well below
        // 1500 ms past the last loud tick).
        entity.perception.audio_rms = Some(0.001);
        entity.tick.now = last_loud_at + (LISTEN_RELEASE_MS / 2);
        let _ = skill.invoke(&mut entity);
        assert!(
            matches!(entity.mind.attention, Attention::Listening { .. }),
            "attention must hold within release window"
        );
        assert_eq!(entity.mind.intent, Intent::Listen);
    }

    #[test]
    fn quiet_past_release_window_clears_attention() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(&mut skill, &mut entity, 0, u64::from(LISTEN_SUSTAIN_TICKS));
        let last_loud_at = entity.tick.now;
        entity.perception.audio_rms = Some(0.001);
        // Step well past the release window.
        entity.tick.now = last_loud_at + LISTEN_RELEASE_MS + 100;
        let _ = skill.invoke(&mut entity);
        assert_eq!(entity.mind.attention, Attention::None);
        assert_eq!(entity.mind.intent, Intent::Idle);
    }

    #[test]
    fn re_entry_after_release_pins_new_since() {
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(&mut skill, &mut entity, 0, u64::from(LISTEN_SUSTAIN_TICKS));
        let Attention::Listening { since: first } = entity.mind.attention else {
            panic!("expected Listening");
        };

        // Quiet past release.
        entity.perception.audio_rms = Some(0.001);
        entity.tick.now = first + LISTEN_RELEASE_MS + 200;
        let _ = skill.invoke(&mut entity);
        assert_eq!(entity.mind.attention, Attention::None);

        // Second sustained burst much later.
        entity.perception.audio_rms = Some(0.3);
        let restart = first + 30_000;
        step(
            &mut skill,
            &mut entity,
            restart.as_millis(),
            u64::from(LISTEN_SUSTAIN_TICKS),
        );
        let Attention::Listening { since: second } = entity.mind.attention else {
            panic!("expected Listening on second burst");
        };
        assert!(
            second > first,
            "second listening attention must use a fresh `since`"
        );
    }

    #[test]
    fn at_threshold_does_not_count_as_loud() {
        // Pin: the predicate is `> threshold`, not `>=`.
        let mut skill = LookAtSound::new();
        let mut entity = Entity::default();
        entity.perception.audio_rms = Some(LISTEN_RMS_THRESHOLD);
        step(
            &mut skill,
            &mut entity,
            0,
            u64::from(LISTEN_SUSTAIN_TICKS) + 5,
        );
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn counter_saturates_does_not_wrap() {
        // 300 loud ticks (well past u8::MAX) should not panic.
        let mut skill = LookAtSound::new();
        let mut entity = loud_avatar();
        step(&mut skill, &mut entity, 0, 300);
        assert!(matches!(entity.mind.attention, Attention::Listening { .. }));
    }
}
