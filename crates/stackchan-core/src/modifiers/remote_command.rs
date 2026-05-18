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
use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::head::Pose;
use crate::input::RemoteCommand;
use crate::mind::{Attention, OverrideSource};
use crate::modifier::Modifier;
use crate::voice::ChirpKind;

/// Tail length re-applied each tick a pairing window is active. Once
/// the window closes the standard
/// [`super::DecoratorExpiry`] sweep clears the overlay after this
/// many milliseconds.
pub const PAIRING_DECORATOR_TAIL_MS: u64 = 500;

/// External control-plane modifier — see module docs for trigger shape.
#[allow(
    clippy::struct_field_names,
    reason = "the `_hold` postfix is the load-bearing distinction across the hold slots"
)]
#[derive(Debug, Default, Clone, Copy)]
pub struct RemoteCommandModifier {
    /// Active emotion hold, if any. `(emotion, hold_until)`.
    emotion_hold: Option<(Emotion, Instant)>,
    /// Active look-at hold, if any. `(target, since, hold_until)`.
    /// `since` is captured at the first frame of the hold so the
    /// rendered ease-in does not restart every tick.
    lookat_hold: Option<(Pose, Instant, Instant)>,
    /// Active 3D look-at-point hold, if any. `(target, since, hold_until)`.
    /// `target` is the raw `(x, y, z)` world point — the modifier graph
    /// does the IK conversion each tick rather than caching the pose,
    /// so a future re-clamp or convention change picks up automatically.
    lookat_point_hold: Option<((f32, f32, f32), Instant, Instant)>,
    /// Active listen hold, if any. `(since, hold_until)`. `since`
    /// pins to the entry frame so listening-pose ease-in animations
    /// don't restart per tick (same idiom as [`Self::lookat_hold`]).
    listen_hold: Option<(Instant, Instant)>,
    /// Active pairing-window deadline, if any. While the timer is
    /// live, [`Self::update`] re-arms [`Decorator::Pairing`] on
    /// `entity.face.decorator` each tick.
    pairing_hold: Option<Instant>,
    /// Active thinking-window hold, if any. `(since, hold_until)`.
    /// `since` pins to the entry frame so [`Attention::Thinking`] is
    /// re-asserted with a stable timestamp each tick (mirrors
    /// [`Self::listen_hold`]). A follow-up `SetEmotion` clears this
    /// hold as a side effect — see [`Self::apply`].
    thinking_hold: Option<(Instant, Instant)>,
}

impl RemoteCommandModifier {
    /// Construct with no active holds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            emotion_hold: None,
            lookat_hold: None,
            lookat_point_hold: None,
            listen_hold: None,
            pairing_hold: None,
            thinking_hold: None,
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
                // Sidecar reply transition: a SetEmotion landing while
                // we're mid-Thinking means the reply has arrived, so
                // the thought-bubble has served its purpose. Clear the
                // hold (the per-tick re-assert below would otherwise
                // hold attention on Thinking through `hold_ms`) and
                // release the attention slot so DecoratorExpiry's tail
                // can fade the bubble out cleanly.
                if let Some((since, _)) = self.thinking_hold.take()
                    && matches!(entity.mind.attention, Attention::Thinking { since: s } if s == since)
                {
                    entity.mind.attention = Attention::None;
                }
            }
            RemoteCommand::LookAt { target, hold_ms } => {
                let until = now + u64::from(hold_ms);
                entity.mind.attention = Attention::Tracking { target, since: now };
                self.lookat_hold = Some((target, now, until));
                // A new 2D look-at supersedes any active 3D point hold.
                self.lookat_point_hold = None;
            }
            RemoteCommand::LookAtPoint { target, hold_ms } => {
                let until = now + u64::from(hold_ms);
                entity.mind.attention = Attention::Point { target, since: now };
                self.lookat_point_hold = Some((target, now, until));
                // A new 3D point supersedes any active 2D look-at hold.
                self.lookat_hold = None;
            }
            RemoteCommand::Reset => {
                entity.mind.autonomy.manual_until = None;
                entity.mind.autonomy.source = None;
                entity.mind.attention = Attention::None;
                self.emotion_hold = None;
                self.lookat_hold = None;
                self.lookat_point_hold = None;
                self.listen_hold = None;
                self.pairing_hold = None;
                self.thinking_hold = None;
            }
            RemoteCommand::Speak { .. } => {
                // Audio dispatch is firmware-only; the producer drains
                // this variant from `entity.input.remote_command` before
                // `Director::run`. If a `Speak` slot survives that
                // intercept, treat it as a no-op rather than panic so
                // the modifier stays resilient under reordering.
            }
            RemoteCommand::StartListen { duration_ms } => {
                let until = now + u64::from(duration_ms);
                entity.mind.attention = Attention::Listening { since: now };
                // Queue an acknowledge chirp on the same tick so the
                // operator gets immediate audible feedback. The firmware
                // audio task drains `voice.chirp_request` per render
                // tick.
                entity.voice.chirp_request = Some(ChirpKind::Wake);
                self.listen_hold = Some((now, until));
                // A stale thinking hold from a previous round-trip
                // (e.g. one that timed out before the operator's hand
                // landed on PTT again) would otherwise stomp this
                // fresh Listening attention every tick. Symmetric
                // with `EnterThinking` clearing `listen_hold` above.
                self.thinking_hold = None;
            }
            RemoteCommand::EnterPairing { duration_ms } => {
                let until = now + u64::from(duration_ms);
                entity.face.decorator = Some(DecoratorState::hold_for(
                    Decorator::Pairing,
                    now,
                    PAIRING_DECORATOR_TAIL_MS,
                ));
                self.pairing_hold = Some(until);
            }
            RemoteCommand::EnterThinking { hold_ms } => {
                let until = now + u64::from(hold_ms);
                entity.mind.attention = Attention::Thinking { since: now };
                self.thinking_hold = Some((now, until));
                // A fresh thinking window supersedes any in-flight
                // listen hold — the listen capture has ended, the
                // round-trip is now the thing we're showing.
                self.listen_hold = None;
            }
            RemoteCommand::ExitThinking => {
                // Off-path cleanup: a successful reply with no emotion
                // tag, a POST failure, or a timeout. Drop the hold and,
                // if attention is still our Thinking, release the slot
                // so `DecoratorExpiry`'s tail can fade the bubble.
                // Same release-guard as the hold-expiry branch in
                // `update` — don't stomp a foreign Thinking attention.
                if let Some((since, _)) = self.thinking_hold.take()
                    && matches!(entity.mind.attention, Attention::Thinking { since: s } if s == since)
                {
                    entity.mind.attention = Attention::None;
                }
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
                Field::ChirpRequest,
                Field::Decorator,
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

        if let Some((target, since, until)) = self.lookat_point_hold {
            if now < until {
                entity.mind.attention = Attention::Point { target, since };
            } else {
                self.lookat_point_hold = None;
                // Only release attention if it's still our point —
                // another modifier may have already taken over.
                if matches!(entity.mind.attention, Attention::Point { target: t, .. } if t == target)
                {
                    entity.mind.attention = Attention::None;
                }
            }
        }

        if let Some((since, until)) = self.listen_hold {
            if now < until {
                entity.mind.attention = Attention::Listening { since };
            } else {
                self.listen_hold = None;
                // Only clear if attention is still our Listening — a
                // tracker observation or another modifier may have
                // already taken over by now.
                if matches!(entity.mind.attention, Attention::Listening { since: s } if s == since)
                {
                    entity.mind.attention = Attention::None;
                }
            }
        }

        if let Some(until) = self.pairing_hold {
            if now < until {
                entity.face.decorator = Some(DecoratorState::hold_for(
                    Decorator::Pairing,
                    now,
                    PAIRING_DECORATOR_TAIL_MS,
                ));
            } else {
                self.pairing_hold = None;
            }
        }

        if let Some((since, until)) = self.thinking_hold {
            if now < until {
                entity.mind.attention = Attention::Thinking { since };
            } else {
                self.thinking_hold = None;
                // Same release-guard idiom as `listen_hold`: only clear
                // if attention is still our Thinking variant — another
                // modifier may have stomped it.
                if matches!(entity.mind.attention, Attention::Thinking { since: s } if s == since) {
                    entity.mind.attention = Attention::None;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic,
    reason = "test-only: f32 fields compared exactly against the literal we wrote; \
              let-else / match-with-panic is the cleanest pattern for value extraction \
              on enum variants in tests; expect on Option is fine in test setup"
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

    #[test]
    fn enter_pairing_arms_decorator_and_refreshes_each_tick() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterPairing { duration_ms: 1_000 });
        step(&mut m, &mut entity, 0);
        let s = entity
            .face
            .decorator
            .expect("Pairing should be armed after EnterPairing");
        assert_eq!(s.kind, Decorator::Pairing);
        assert_eq!(
            s.expires_at,
            Instant::from_millis(PAIRING_DECORATOR_TAIL_MS)
        );

        // Advance mid-window: expiry refreshes to `now + tail`.
        step(&mut m, &mut entity, 500);
        let s = entity.face.decorator.expect("still armed mid-window");
        assert_eq!(s.kind, Decorator::Pairing);
        assert_eq!(
            s.expires_at,
            Instant::from_millis(500 + PAIRING_DECORATOR_TAIL_MS)
        );
    }

    #[test]
    fn enter_pairing_stops_refreshing_after_window_expires() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterPairing { duration_ms: 100 });
        step(&mut m, &mut entity, 0);
        // After the 100 ms window the modifier stops re-arming. The
        // overlay clears via DecoratorExpiry once the 500 ms tail elapses;
        // the modifier itself just stops touching the field.
        let pre_expiry = entity.face.decorator;
        step(&mut m, &mut entity, 1_000);
        // The previous DecoratorState may still sit in the field if no
        // expiry sweep has run; the assertion that matters is that the
        // pairing_hold is released.
        assert_eq!(entity.face.decorator, pre_expiry);
    }

    #[test]
    fn reset_clears_pairing_hold() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterPairing {
            duration_ms: 30_000,
        });
        step(&mut m, &mut entity, 0);
        assert!(entity.face.decorator.is_some());

        entity.input.remote_command = Some(RemoteCommand::Reset);
        step(&mut m, &mut entity, 100);
        // Reset clears the timer; the decorator field is left alone
        // (DecoratorExpiry handles the visual fade) — what we pin here
        // is that subsequent ticks no longer refresh the overlay.
        let after_reset = entity.face.decorator;
        step(&mut m, &mut entity, 200);
        assert_eq!(entity.face.decorator, after_reset);
    }

    #[test]
    fn enter_thinking_sets_attention_and_pins_since() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(100);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 5_000 });
        step(&mut m, &mut entity, 100);

        match entity.mind.attention {
            Attention::Thinking { since } => {
                assert_eq!(since, Instant::from_millis(100));
            }
            other => panic!("expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn thinking_hold_re_asserts_against_stomping() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 1_000 });
        step(&mut m, &mut entity, 0);

        entity.mind.attention = Attention::None;
        step(&mut m, &mut entity, 200);
        assert!(
            matches!(entity.mind.attention, Attention::Thinking { .. }),
            "hold must re-assert Thinking against mid-frame stomps"
        );
    }

    #[test]
    fn thinking_hold_expires_back_to_none() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 500 });
        step(&mut m, &mut entity, 0);
        step(&mut m, &mut entity, 600);
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn set_emotion_clears_active_thinking() {
        // Reply-arrival side effect: a SetEmotion landing mid-Thinking
        // means the sidecar reply has come back, so the thought-bubble
        // has served its purpose. Attention drops to None on the same
        // tick; the emotion + speech-bubble carry the visible reply.
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 10_000 });
        step(&mut m, &mut entity, 0);
        assert!(matches!(entity.mind.attention, Attention::Thinking { .. }));

        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 2_000,
        });
        step(&mut m, &mut entity, 500);

        assert_eq!(entity.mind.attention, Attention::None);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    }

    #[test]
    fn set_emotion_does_not_clear_unrelated_thinking_attention() {
        // If something else has written Attention::Thinking with a
        // different `since`, SetEmotion's clear is scoped to our hold.
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 10_000 });
        step(&mut m, &mut entity, 0);

        let foreign_since = Instant::from_millis(999);
        entity.mind.attention = Attention::Thinking {
            since: foreign_since,
        };
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 2_000,
        });
        step(&mut m, &mut entity, 500);

        // The foreign Thinking attention survives the SetEmotion.
        assert_eq!(
            entity.mind.attention,
            Attention::Thinking {
                since: foreign_since,
            }
        );
    }

    #[test]
    fn enter_thinking_supersedes_listen_hold() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::StartListen {
            duration_ms: 30_000,
        });
        step(&mut m, &mut entity, 0);
        assert!(matches!(entity.mind.attention, Attention::Listening { .. }));

        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 5_000 });
        step(&mut m, &mut entity, 100);
        assert!(matches!(entity.mind.attention, Attention::Thinking { .. }));

        // The listen hold must not resurrect Listening on the next tick.
        step(&mut m, &mut entity, 200);
        assert!(matches!(entity.mind.attention, Attention::Thinking { .. }));
    }

    #[test]
    fn reset_clears_thinking_hold() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 30_000 });
        step(&mut m, &mut entity, 0);
        assert!(matches!(entity.mind.attention, Attention::Thinking { .. }));

        entity.input.remote_command = Some(RemoteCommand::Reset);
        step(&mut m, &mut entity, 100);
        assert_eq!(entity.mind.attention, Attention::None);

        // Subsequent ticks do not resurrect Thinking.
        step(&mut m, &mut entity, 500);
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn exit_thinking_clears_attention_without_touching_emotion() {
        // Off-path cleanup: a no-emotion success or a failure path
        // fires ExitThinking. The operator's existing emotion hold
        // (set via dashboard) must survive — only the thinking slot
        // is released.
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::SetEmotion {
            emotion: Emotion::Happy,
            hold_ms: 60_000,
        });
        step(&mut m, &mut entity, 0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 15_000 });
        step(&mut m, &mut entity, 100);

        entity.input.remote_command = Some(RemoteCommand::ExitThinking);
        step(&mut m, &mut entity, 200);
        assert_eq!(entity.mind.attention, Attention::None);
        // Operator's emotion hold from t=0 survives the off-path clear.
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(
            entity.mind.autonomy.source,
            Some(OverrideSource::Remote),
            "emotion override must remain after ExitThinking"
        );

        // No resurrection on subsequent ticks.
        step(&mut m, &mut entity, 500);
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn exit_thinking_is_noop_when_no_thinking_hold() {
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::ExitThinking);
        step(&mut m, &mut entity, 0);
        assert_eq!(entity.mind.attention, Attention::None);
    }

    #[test]
    fn start_listen_clears_stale_thinking_hold() {
        // Scenario: the prior round-trip timed out and left
        // thinking_hold live. The operator hits PTT again before the
        // 15s hold expires. The new Listening capture must read as
        // Ear, not as a stomped-thought-bubble.
        let mut m = RemoteCommandModifier::new();
        let mut entity = entity_at(0);
        entity.input.remote_command = Some(RemoteCommand::EnterThinking { hold_ms: 15_000 });
        step(&mut m, &mut entity, 0);
        assert!(matches!(entity.mind.attention, Attention::Thinking { .. }));

        // Fresh PTT mid-thinking-hold.
        entity.input.remote_command = Some(RemoteCommand::StartListen { duration_ms: 3_000 });
        step(&mut m, &mut entity, 5_000);
        assert!(matches!(entity.mind.attention, Attention::Listening { .. }));

        // The next tick must not resurrect Thinking from the stale
        // hold — symmetric with the supersede check that EnterThinking
        // does on listen_hold.
        step(&mut m, &mut entity, 5_100);
        assert!(matches!(entity.mind.attention, Attention::Listening { .. }));
    }
}
