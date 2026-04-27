//! `RemoteCommandModifier`: external control-plane commands assert
//! emotion or attention with a hold timer.
//!
//! ## Trigger shape
//!
//! Reads `entity.input.remote_command` set by the firmware HTTP task
//! (or any other producer). Three command shapes:
//!
//! - [`RemoteCommand::SetEmotion`] — writes `mind.affect.emotion` and
//!   pins `mind.autonomy.manual_until = now + hold_ms` with
//!   [`OverrideSource::Remote`]. Autonomous emotion drivers in
//!   [`Phase::Affect`] (which runs after [`Phase::Cognition`]) gate
//!   on `manual_until` and stand down for the hold's duration —
//!   same idiom as
//!   [`super::EmotionFromRemote`].
//! - [`RemoteCommand::LookAt`] — writes
//!   `mind.attention = Attention::Tracking { target, since: now }`
//!   and stashes a hold timer. While the hold is active the modifier
//!   re-asserts the same target each tick at higher priority than
//!   [`super::AttentionFromTracking`], so a face entering the frame
//!   mid-hold cannot stomp the operator's target. `since` is pinned
//!   to the entry frame so consumer ease-in animations stay smooth.
//! - [`RemoteCommand::Reset`] — clears any active emotion or look-at
//!   hold and resets `mind.autonomy.manual_until` /
//!   `mind.attention` to defaults.
//!
//! ## Why a Modifier, not a Skill
//!
//! [`Skill`](crate::Skill) writes are framework-restricted to
//! [`Mind`](crate::director::FieldGroup::Mind) /
//! [`Voice`](crate::director::FieldGroup::Voice) — a skill can't
//! drain [`Input::remote_command`](crate::Input::remote_command).
//! Modifiers can. The hold-timer state lives inside the modifier the
//! same way [`super::AttentionFromTracking`] holds its lock counter.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::head::Pose;
use crate::input::RemoteCommand;
use crate::mind::{Attention, OverrideSource};
use crate::modifier::Modifier;

/// External control-plane modifier — see module docs for trigger shape.
#[derive(Debug, Default, Clone, Copy)]
pub struct RemoteCommandModifier {
    /// Active emotion hold, if any. `(emotion, hold_until)`.
    emotion_hold: Option<(Emotion, Instant)>,
    /// Active look-at hold, if any. `(target, since, hold_until)`.
    /// `since` is captured at the first frame of the hold so the
    /// rendered ease-in does not restart every tick.
    lookat_hold: Option<(Pose, Instant, Instant)>,
}

impl RemoteCommandModifier {
    /// Construct with no active holds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            emotion_hold: None,
            lookat_hold: None,
        }
    }

    /// Apply a freshly received command: write the matching mind
    /// fields and stash any hold timer for re-assertion in subsequent
    /// ticks.
    fn apply(&mut self, command: RemoteCommand, now: Instant, entity: &mut Entity) {
        match command {
            RemoteCommand::SetEmotion { emotion, hold_ms } => {
                let until = now + u64::from(hold_ms);
                entity.mind.affect.emotion = emotion;
                entity.mind.autonomy.manual_until = Some(until);
                entity.mind.autonomy.source = Some(OverrideSource::Remote);
                self.emotion_hold = Some((emotion, until));
            }
            RemoteCommand::LookAt { target, hold_ms } => {
                let until = now + u64::from(hold_ms);
                entity.mind.attention = Attention::Tracking { target, since: now };
                self.lookat_hold = Some((target, now, until));
            }
            RemoteCommand::Reset => {
                entity.mind.autonomy.manual_until = None;
                entity.mind.autonomy.source = None;
                entity.mind.attention = Attention::None;
                self.emotion_hold = None;
                self.lookat_hold = None;
            }
            RemoteCommand::Speak { .. } => {
                // Audio dispatch is firmware-only; the producer drains
                // this variant from `entity.input.remote_command` before
                // `Director::run`. If a `Speak` slot survives that
                // intercept, treat it as a no-op rather than panic so
                // the modifier stays resilient under reordering.
            }
        }
    }
}

impl Modifier for RemoteCommandModifier {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "RemoteCommandModifier",
            description: "Drains entity.input.remote_command into mind.affect.emotion + \
                          mind.autonomy (SetEmotion) or mind.attention (LookAt) and re-asserts \
                          the value each tick until the hold timer expires. Reset clears all \
                          active holds. Priority 100 in Phase::Cognition runs after \
                          AttentionFromTracking so a tracking observation cannot stomp the \
                          operator's target during a hold.",
            phase: Phase::Cognition,
            priority: 100,
            reads: &[Field::RemoteCommand, Field::Autonomy, Field::Attention],
            writes: &[
                Field::Emotion,
                Field::Autonomy,
                Field::Attention,
                Field::RemoteCommand,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        if let Some(command) = entity.input.remote_command.take() {
            self.apply(command, now, entity);
        }

        if let Some((emotion, until)) = self.emotion_hold {
            if now < until {
                entity.mind.affect.emotion = emotion;
                entity.mind.autonomy.manual_until = Some(until);
                entity.mind.autonomy.source = Some(OverrideSource::Remote);
            } else {
                self.emotion_hold = None;
                if entity.mind.autonomy.source == Some(OverrideSource::Remote) {
                    entity.mind.autonomy.manual_until = None;
                    entity.mind.autonomy.source = None;
                }
            }
        }

        if let Some((target, since, until)) = self.lookat_hold {
            if now < until {
                entity.mind.attention = Attention::Tracking { target, since };
            } else {
                self.lookat_hold = None;
                if matches!(entity.mind.attention, Attention::Tracking { target: t, .. } if t == target)
                {
                    entity.mind.attention = Attention::None;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::panic,
    reason = "test-only: f32 fields compared exactly against the literal we wrote; \
              let-else / match-with-panic is the cleanest pattern for value extraction \
              on enum variants in tests"
)]
mod tests {
    use super::*;
    use crate::Affect;
    use crate::mind::Autonomy;

    fn entity_at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    fn step(modifier: &mut RemoteCommandModifier, entity: &mut Entity, now_ms: u64) {
        entity.tick.now = Instant::from_millis(now_ms);
        modifier.update(entity);
    }

    #[test]
    fn set_emotion_writes_emotion_and_autonomy() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 1_000,
        });

        step(&mut m, &mut entity, 0);

        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(1_000))
        );
        assert_eq!(entity.mind.autonomy.source, Some(OverrideSource::Remote));
        assert!(entity.input.remote_command.is_none());
    }

    #[test]
    fn emotion_hold_re_asserts_each_tick_against_stomping() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 1_000,
        });
        step(&mut m, &mut entity, 0);

        entity.mind.affect = Affect {
            emotion: Emotion::Sleepy,
        };
        step(&mut m, &mut entity, 100);
        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Happy,
            "hold must re-assert against mid-frame stomps"
        );
    }

    #[test]
    fn emotion_hold_releases_after_timer_expires() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 500,
        });
        step(&mut m, &mut entity, 0);
        step(&mut m, &mut entity, 600);

        assert!(entity.mind.autonomy.manual_until.is_none());
        assert!(entity.mind.autonomy.source.is_none());
    }

    #[test]
    fn emotion_release_does_not_clear_a_different_owner() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 100,
        });
        step(&mut m, &mut entity, 0);

        entity.mind.autonomy = Autonomy {
            manual_until: Some(Instant::from_millis(10_000)),
            source: Some(OverrideSource::LowBattery),
        };
        step(&mut m, &mut entity, 200);

        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(10_000))
        );
        assert_eq!(
            entity.mind.autonomy.source,
            Some(OverrideSource::LowBattery)
        );
    }

    #[test]
    fn lookat_writes_attention_with_since_pinned_to_entry() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(5_000);
        entity.input.remote_command = Some(RemoteCommand::LookAt {
            target: Pose {
                pan_deg: 12.0,
                tilt_deg: -3.0,
            },
            hold_ms: 1_000,
        });

        step(&mut m, &mut entity, 5_000);

        let entry = match entity.mind.attention {
            Attention::Tracking { target, since } => {
                assert_eq!(target.pan_deg, 12.0);
                assert_eq!(target.tilt_deg, -3.0);
                since
            }
            other => panic!("expected Tracking, got {other:?}"),
        };

        step(&mut m, &mut entity, 5_033);
        step(&mut m, &mut entity, 5_066);
        match entity.mind.attention {
            Attention::Tracking { since, .. } => {
                assert_eq!(since, entry, "since must pin to entry frame");
            }
            other => panic!("expected Tracking still, got {other:?}"),
        }
    }

    #[test]
    fn lookat_hold_re_asserts_against_tracking_mid_hold() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::LookAt {
            target: Pose {
                pan_deg: 20.0,
                tilt_deg: 0.0,
            },
            hold_ms: 1_000,
        });
        step(&mut m, &mut entity, 0);

        entity.mind.attention = Attention::Tracking {
            target: Pose {
                pan_deg: -45.0,
                tilt_deg: 10.0,
            },
            since: Instant::from_millis(100),
        };

        step(&mut m, &mut entity, 100);
        match entity.mind.attention {
            Attention::Tracking { target, .. } => {
                assert_eq!(
                    target.pan_deg, 20.0,
                    "remote target must override tracking during hold"
                );
            }
            other => panic!("expected Tracking, got {other:?}"),
        }
    }

    #[test]
    fn lookat_release_clears_when_target_unchanged() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        let target = Pose {
            pan_deg: 5.0,
            tilt_deg: 0.0,
        };
        entity.input.remote_command = Some(RemoteCommand::LookAt {
            target,
            hold_ms: 200,
        });
        step(&mut m, &mut entity, 0);

        step(&mut m, &mut entity, 300);
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn lookat_release_does_not_clear_a_different_target() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::LookAt {
            target: Pose {
                pan_deg: 5.0,
                tilt_deg: 0.0,
            },
            hold_ms: 100,
        });
        step(&mut m, &mut entity, 0);

        let face_target = Pose {
            pan_deg: -30.0,
            tilt_deg: 5.0,
        };
        entity.mind.attention = Attention::Tracking {
            target: face_target,
            since: Instant::from_millis(150),
        };
        step(&mut m, &mut entity, 200);

        match entity.mind.attention {
            Attention::Tracking { target, .. } => {
                assert_eq!(
                    target, face_target,
                    "release must not clobber a fresh tracking target"
                );
            }
            other => panic!("expected fresh Tracking, got {other:?}"),
        }
    }

    #[test]
    fn reset_clears_both_holds_and_returns_to_default() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Angry,
            hold_ms: 10_000,
        });
        step(&mut m, &mut entity, 0);
        entity.input.remote_command = Some(RemoteCommand::LookAt {
            target: Pose {
                pan_deg: 12.0,
                tilt_deg: 0.0,
            },
            hold_ms: 10_000,
        });
        step(&mut m, &mut entity, 50);

        entity.input.remote_command = Some(RemoteCommand::Reset);
        step(&mut m, &mut entity, 100);

        assert!(entity.mind.autonomy.manual_until.is_none());
        assert!(entity.mind.autonomy.source.is_none());
        assert_eq!(entity.mind.attention, Attention::None);

        entity.mind.affect = Affect {
            emotion: Emotion::Sleepy,
        };
        step(&mut m, &mut entity, 200);
        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Sleepy,
            "reset must drop the emotion hold"
        );
    }

    #[test]
    fn speak_is_a_no_op_at_the_modifier() {
        // Speak is dispatched to the audio queue by the firmware
        // before Director::run. If a Speak slot reaches the modifier,
        // it must consume harmlessly without touching mind state.
        use crate::voice::{Locale, PhraseId, Priority};
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        let baseline_emotion = entity.mind.affect.emotion;
        let baseline_attention = entity.mind.attention;
        entity.input.remote_command = Some(RemoteCommand::Speak {
            phrase: PhraseId::WakeChirp,
            locale: Locale::En,
            priority: Priority::Normal,
        });

        step(&mut m, &mut entity, 0);

        assert_eq!(entity.mind.affect.emotion, baseline_emotion);
        assert_eq!(entity.mind.attention, baseline_attention);
        assert!(entity.input.remote_command.is_none(), "slot must drain");
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn zero_hold_ms_is_fire_and_forget() {
        // hold_ms=0 sets emotion + autonomy, then the same-tick
        // re-assert sees `now < now == false` and releases the
        // autonomy. Operators who want a sticky override pass a
        // non-zero hold_ms.
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 0,
        });
        step(&mut m, &mut entity, 0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }
}
