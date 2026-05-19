//! [`DancePlayer`] — sample a [`DanceScript`] each tick and apply
//! the active keyframes to head pose, emotion, decorator, and LED
//! override.
//!
//! ## Architecture
//!
//! Operator loads a script via `dance_player.load_script(script,
//! now)`; the player anchors on `now` and starts ticking. Each tick:
//!
//! 1. Compute `elapsed = now - started_at`.
//! 2. For each channel (motion / emotion / decorator / RGB) find the
//!    most-recent keyframe at-or-before `elapsed` carrying that channel.
//! 3. Apply that keyframe's value to the entity (with diff-and-undo
//!    on `motor.head_pose` to compose with the rest of the Motion stack).
//! 4. When `elapsed` exceeds the last keyframe + [`SCRIPT_TAIL_MS`],
//!    clear all overrides and drop the script.
//!
//! The player runs in [`Phase::Motion`] at priority `40` — last in the
//! Motion phase, so the dance overrides idle drift / emotion bias /
//! attention follow / startle recoil / pet reaction. Operator intent
//! ("play this script now") trumps autonomous behaviour.
//!
//! ## Cross-phase writes
//!
//! `entity.mind.affect.emotion` is normally written by `Phase::Affect`
//! modifiers; the player writes it in `Phase::Motion`, one phase
//! later. The render task reads emotion at the *end* of the frame
//! (for the next frame's draw), so the cross-phase write produces a
//! one-frame visual lag — invisible at 30 FPS. This trade-off keeps
//! the dance state machine in a single modifier rather than three
//! coordinated modifiers (one per phase).
//!
//! ## Autonomy
//!
//! While a script holds an `emotion` override the player also pins
//! `mind.autonomy.manual_until` so `EmotionCycle` can't advance the
//! emotion mid-dance. Source is recorded as
//! [`crate::OverrideSource::Remote`] — same as HTTP `/emotion`.

use alloc::sync::Arc;

use crate::clock::Instant;
use crate::dance::{DanceScript, Keyframe, SCRIPT_TAIL_MS};
use crate::decorator::{Decorator, DecoratorState};
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::head::Pose;
use crate::mind::OverrideSource;
use crate::modifier::Modifier;

/// Hold duration applied to `mind.autonomy.manual_until` while a
/// script's emotion override is active, in milliseconds.
///
/// Refreshed each tick the player asserts an emotion, so any pause
/// in emotion-keyframe coverage releases autonomy gracefully.
pub const DANCE_AUTONOMY_HOLD_MS: u64 = 200;

/// Player state for one dance script.
///
/// Holds the script in an [`Arc`] so the firmware can hand off
/// scripts via a Signal without copying the (potentially long)
/// keyframe vector.
#[derive(Debug, Clone)]
pub struct DancePlayer {
    /// Active script + the wall-clock instant the script started.
    /// `None` when no script is loaded.
    active: Option<ActiveScript>,
    /// Pose contribution applied on the previous tick (post-clamp).
    /// Subtracted before the new contribution lands so the dance
    /// composes additively over the rest of the Motion stack.
    last_pan_deg: f32,
    /// Tilt counterpart of [`Self::last_pan_deg`].
    last_tilt_deg: f32,
    /// Whether the player wrote a non-`None` emotion / decorator /
    /// `led_override` on the previous tick. Tracked so the cleanup pass
    /// at script end clears only the channels the player owned.
    held_emotion: bool,
    /// See [`Self::held_emotion`].
    held_decorator: bool,
    /// See [`Self::held_emotion`].
    held_led: bool,
}

/// Currently-loaded script + anchor instant.
#[derive(Debug, Clone)]
struct ActiveScript {
    /// Script data, refcount-shared with the loader so the firmware
    /// can hand it through a Signal channel cheaply.
    script: Arc<DanceScript>,
    /// Wall-clock instant the script began. `now - started_at` is
    /// the keyframe sample offset.
    started_at: Instant,
}

impl DancePlayer {
    /// Construct an idle player.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
            held_emotion: false,
            held_decorator: false,
            held_led: false,
        }
    }

    /// Load a fresh script. `started_at` is the wall-clock instant
    /// from which keyframe `at_ms` offsets are measured. Replacing
    /// an active script just re-anchors at `started_at`; the old
    /// script's overrides are cleared on the next tick by the
    /// per-channel diff against the new sample.
    pub fn load_script(&mut self, script: Arc<DanceScript>, started_at: Instant) {
        self.active = Some(ActiveScript { script, started_at });
    }

    /// `true` if the player currently has a script loaded. Operator-
    /// surface helper for HTTP `GET /dance/state` (future).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

impl Default for DancePlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for DancePlayer {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DancePlayer",
            description: "Samples a loaded DanceScript each tick and overrides motor.head_pose \
                          (motion channel), mind.affect.emotion + mind.autonomy (avatar emotion), \
                          face.decorator (avatar decorator), and led_override (RGB channel) per \
                          keyframe. Composes additively after the rest of the Motion stack via \
                          diff-and-undo on the head pose contribution.",
            phase: Phase::Motion,
            priority: 40,
            reads: &[Field::HeadPose, Field::DanceScript],
            writes: &[
                Field::HeadPose,
                Field::Emotion,
                Field::Autonomy,
                Field::Decorator,
                Field::LedOverride,
                // Drained on the load tick — see `update`.
                Field::DanceScript,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        // Drain any pending script from `entity.input` first so an
        // upload landing this tick takes effect immediately rather
        // than waiting one frame.
        if let Some(script) = entity.input.dance_script.take() {
            self.active = Some(ActiveScript {
                script,
                started_at: now,
            });
        }
        let Some(active) = self.active.as_ref() else {
            // No script — diff-and-undo any leftover head contribution
            // from a prior tick so the upstream pose passes through.
            self.release_head(entity);
            return;
        };

        let elapsed = now.saturating_duration_since(active.started_at);
        // Scripts longer than ~49 days truncate the offset (wrapping
        // cast) and re-sample from the beginning of the keyframe
        // list. Acceptable — anything close to that limit is well
        // outside the design envelope.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "scripts are bounded by MAX_KEYFRAMES * dance length; truncation here is benign"
        )]
        let elapsed_ms = elapsed as u32;
        let last_at_ms = active.script.keyframes.last().map_or(0, |kf| kf.at_ms);

        if elapsed_ms > last_at_ms.saturating_add(SCRIPT_TAIL_MS) {
            // Script complete. Clear our overrides on emotion /
            // decorator / led, then release head and drop the script.
            self.release_overrides(entity);
            self.release_head(entity);
            self.active = None;
            return;
        }

        // Sample each channel. None means "no keyframe yet covers
        // this channel" — the player holds the prior held value if
        // any (carried across ticks via the entity field), or leaves
        // upstream alone if nothing has fired yet for that channel.
        let sample = sample_channels(&active.script.keyframes, elapsed_ms);

        // Motion channel — additive diff-and-undo composition.
        let target_pose = sample.pose;
        self.apply_head(entity, target_pose);

        // Avatar emotion — pin until next tick. Refresh the
        // autonomy hold each tick we hold an emotion so the autonomous
        // cycler stays deferred for the whole dance window.
        if let Some(emotion) = sample.emotion {
            entity.mind.affect.emotion = emotion;
            entity.mind.autonomy.manual_until = Some(now + DANCE_AUTONOMY_HOLD_MS);
            entity.mind.autonomy.source = Some(OverrideSource::Remote);
            self.held_emotion = true;
        }

        // Avatar decorator — write directly. The Decoration phase's
        // expiry modifier honors `expires_at`, so we set a generous
        // expiry that covers the next keyframe + tail.
        if let Some(kind) = sample.decorator {
            entity.face.decorator =
                Some(DecoratorState::hold_for(kind, now, DANCE_AUTONOMY_HOLD_MS));
            self.held_decorator = true;
        }

        // RGB channel — write the [r, g, b] triple verbatim.
        if let Some(rgb) = sample.led {
            entity.led_override = Some(rgb);
            self.held_led = true;
        } else if self.held_led && entity.led_override.is_some() {
            // No RGB sample this tick AND we owned the previous
            // override — release.
            entity.led_override = None;
            self.held_led = false;
        }
    }
}

impl DancePlayer {
    /// Apply a target pose with diff-and-undo composition.
    ///
    /// Additive layering, not replacement: the dance offset rides
    /// on top of whatever the upstream Motion stack produced
    /// (`HeadFromEmotion`'s emotion tilt, `HeadFromAttention`'s
    /// listening lift, etc.). Choreographers can compensate by
    /// pinning emotion to `Neutral` in the script's first
    /// keyframe — that zeros out the emotion-driven bias for the
    /// duration of the dance.
    ///
    /// The diff-and-undo dance: subtract our previous applied
    /// contribution to recover the true upstream, add the new
    /// target on top, clamp, store the post-clamp effective delta
    /// for next tick's recovery.
    fn apply_head(&mut self, entity: &mut Entity, target: Pose) {
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;
        let combined = Pose::new(
            upstream_pan + target.pan_deg,
            upstream_tilt + target.tilt_deg,
        )
        .clamped();
        self.last_pan_deg = combined.pan_deg - upstream_pan;
        self.last_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }

    /// Undo the previous tick's head contribution so upstream
    /// modifiers' work is what the head task sees this tick.
    fn release_head(&mut self, entity: &mut Entity) {
        if self.last_pan_deg == 0.0 && self.last_tilt_deg == 0.0 {
            return;
        }
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;
        entity.motor.head_pose = Pose::new(upstream_pan, upstream_tilt).clamped();
        self.last_pan_deg = 0.0;
        self.last_tilt_deg = 0.0;
    }

    /// Clear emotion / decorator / led overrides we previously held.
    #[allow(
        clippy::missing_const_for_fn,
        reason = "future revisions will conditionally check OverrideSource origin before clearing"
    )]
    fn release_overrides(&mut self, entity: &mut Entity) {
        if self.held_emotion {
            // Release the autonomy gate only if no other override
            // source took over while the dance held.
            if matches!(entity.mind.autonomy.source, Some(OverrideSource::Remote)) {
                entity.mind.autonomy.manual_until = None;
                entity.mind.autonomy.source = None;
            }
            self.held_emotion = false;
        }
        if self.held_decorator {
            entity.face.decorator = None;
            self.held_decorator = false;
        }
        if self.held_led {
            entity.led_override = None;
            self.held_led = false;
        }
    }
}

/// Per-channel sample at a given offset.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct ChannelSample {
    /// Most-recent (pan, tilt) from any keyframe with at least one
    /// motion field set. Defaults to neutral if no keyframe has
    /// fired for this channel yet.
    pose: Pose,
    /// Most-recent emotion override, or `None` if no keyframe has
    /// fired for this channel yet.
    emotion: Option<Emotion>,
    /// Most-recent decorator override.
    decorator: Option<Decorator>,
    /// Most-recent RGB triple.
    led: Option<[u8; 3]>,
}

/// Walk keyframes up to `elapsed_ms` and pick the most-recent value
/// for each channel. Linear scan — for thousand-frame scripts this
/// is one pass per render tick (~33 µs at 30 FPS).
fn sample_channels(keyframes: &[Keyframe], elapsed_ms: u32) -> ChannelSample {
    let mut sample = ChannelSample::default();
    let mut motion_pan: Option<f32> = None;
    let mut motion_tilt: Option<f32> = None;
    for kf in keyframes {
        if kf.at_ms > elapsed_ms {
            break;
        }
        if let Some(p) = kf.pan_deg {
            motion_pan = Some(p);
        }
        if let Some(t) = kf.tilt_deg {
            motion_tilt = Some(t);
        }
        if let Some(e) = kf.emotion {
            sample.emotion = Some(e);
        }
        if let Some(d) = kf.decorator {
            sample.decorator = Some(d);
        }
        if let Some(triple) = kf.rgb() {
            sample.led = Some([triple.0, triple.1, triple.2]);
        }
    }
    sample.pose = Pose::new(motion_pan.unwrap_or(0.0), motion_tilt.unwrap_or(0.0));
    sample
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: bit-exact assertions on our own sampling math"
)]
mod tests {
    use super::*;
    use alloc::vec;

    fn make_script(keyframes: alloc::vec::Vec<Keyframe>) -> Arc<DanceScript> {
        Arc::new(DanceScript { keyframes })
    }

    fn entity_at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    #[test]
    fn idle_player_writes_nothing() {
        let mut player = DancePlayer::new();
        let mut entity = entity_at(100);
        let upstream = Pose::new(2.0, 5.0);
        entity.motor.head_pose = upstream;
        player.update(&mut entity);
        assert_eq!(entity.motor.head_pose, upstream);
        assert_eq!(entity.led_override, None);
    }

    #[test]
    fn loaded_script_writes_first_keyframe_at_t0() {
        let script = make_script(vec![Keyframe {
            at_ms: 0,
            pan_deg: Some(15.0),
            tilt_deg: Some(10.0),
            emotion: Some(Emotion::Happy),
            r: Some(255),
            g: Some(100),
            b: Some(0),
            ..Keyframe::default()
        }]);
        let mut player = DancePlayer::new();
        player.load_script(script, Instant::from_millis(1_000));

        let mut entity = entity_at(1_000);
        // Upstream pose at zero so the additive layering produces
        // exactly the keyframe target on the head.
        player.update(&mut entity);

        assert_eq!(entity.motor.head_pose.pan_deg, 15.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 10.0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(entity.led_override, Some([255, 100, 0]));
    }

    #[test]
    fn player_holds_unset_channels_at_prior_value() {
        // Keyframe 0 sets all channels; keyframe 500 sets only pan.
        // At t=600 the player should hold the original emotion and
        // led from keyframe 0 while pan tracks keyframe 500.
        let script = make_script(vec![
            Keyframe {
                at_ms: 0,
                pan_deg: Some(0.0),
                emotion: Some(Emotion::Happy),
                r: Some(10),
                g: Some(20),
                b: Some(30),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 500,
                pan_deg: Some(20.0),
                ..Keyframe::default()
            },
        ]);
        let mut player = DancePlayer::new();
        player.load_script(script, Instant::from_millis(0));

        let mut entity = entity_at(600);
        player.update(&mut entity);

        assert_eq!(entity.motor.head_pose.pan_deg, 20.0);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(entity.led_override, Some([10, 20, 30]));
    }

    #[test]
    fn script_completion_releases_overrides() {
        let script = make_script(vec![Keyframe {
            at_ms: 0,
            emotion: Some(Emotion::Happy),
            r: Some(255),
            g: Some(0),
            b: Some(0),
            ..Keyframe::default()
        }]);
        let mut player = DancePlayer::new();
        player.load_script(script, Instant::from_millis(0));

        // First tick: overrides applied.
        let mut entity = entity_at(0);
        player.update(&mut entity);
        assert_eq!(entity.led_override, Some([255, 0, 0]));

        // Past last keyframe + SCRIPT_TAIL_MS: overrides released.
        entity.tick.now = Instant::from_millis(u64::from(SCRIPT_TAIL_MS) + 100);
        player.update(&mut entity);
        assert_eq!(entity.led_override, None);
        assert!(!player.is_active());
    }

    #[test]
    fn additive_composition_layers_target_on_upstream() {
        let script = make_script(vec![Keyframe {
            at_ms: 0,
            pan_deg: Some(15.0),
            tilt_deg: Some(10.0),
            ..Keyframe::default()
        }]);
        let mut player = DancePlayer::new();
        player.load_script(script, Instant::from_millis(0));

        let mut entity = entity_at(0);
        // Upstream wrote a pose this frame; dance target rides on top.
        entity.motor.head_pose = Pose::new(2.0, 5.0);
        player.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 17.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 15.0);
    }

    #[test]
    fn diff_and_undo_lets_upstream_resume_after_release() {
        let script = make_script(vec![Keyframe {
            at_ms: 0,
            pan_deg: Some(15.0),
            tilt_deg: Some(10.0),
            ..Keyframe::default()
        }]);
        let mut player = DancePlayer::new();
        player.load_script(script, Instant::from_millis(0));

        // Tick 1: upstream + dance contribution land on head_pose.
        let mut entity = entity_at(0);
        entity.motor.head_pose = Pose::new(2.0, 5.0);
        player.update(&mut entity);
        // 2 + 15 = 17 pan; 5 + 10 = 15 tilt.
        assert_eq!(entity.motor.head_pose.pan_deg, 17.0);

        // Tick 2 (script ended): firmware re-applies upstream by
        // running the rest of the Motion stack first; head_pose at
        // start of DancePlayer.update reflects upstream + dance's
        // last applied delta. Diff-and-undo recovers true upstream.
        let dance_pan_delta = entity.motor.head_pose.pan_deg - 2.0;
        let dance_tilt_delta = entity.motor.head_pose.tilt_deg - 5.0;
        // Simulate the firmware re-applying upstream on top of the
        // prior tick's combined pose: head_pose still carries the
        // dance delta because nothing reset it between Director::run
        // calls.
        entity.tick.now = Instant::from_millis(u64::from(SCRIPT_TAIL_MS) + 100);
        // The next firmware tick: upstream-Motion modifiers run with
        // their own diff-and-undo; the dance's delta from last tick
        // is still embedded in head_pose. We model that here.
        let _ = (dance_pan_delta, dance_tilt_delta);
        player.update(&mut entity);
        // Past the tail, player should release; head_pose returns to
        // pure upstream.
        assert!((entity.motor.head_pose.pan_deg - 2.0).abs() < 0.001);
        assert!((entity.motor.head_pose.tilt_deg - 5.0).abs() < 0.001);
    }

    #[test]
    fn sample_channels_picks_most_recent_per_channel() {
        let kfs = vec![
            Keyframe {
                at_ms: 0,
                pan_deg: Some(-10.0),
                emotion: Some(Emotion::Sleepy),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 100,
                pan_deg: Some(20.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 200,
                emotion: Some(Emotion::Happy),
                ..Keyframe::default()
            },
        ];
        let s = sample_channels(&kfs, 150);
        // pan tracks the t=100 update; emotion still on t=0 since
        // t=200 hasn't fired yet at elapsed=150.
        assert_eq!(s.pose.pan_deg, 20.0);
        assert_eq!(s.emotion, Some(Emotion::Sleepy));

        let s = sample_channels(&kfs, 250);
        // Now both have fired; emotion advances.
        assert_eq!(s.pose.pan_deg, 20.0);
        assert_eq!(s.emotion, Some(Emotion::Happy));
    }

    #[test]
    fn sample_before_first_keyframe_returns_default_pose() {
        let kfs = vec![Keyframe {
            at_ms: 100,
            pan_deg: Some(20.0),
            ..Keyframe::default()
        }];
        let s = sample_channels(&kfs, 50);
        assert_eq!(s.pose, Pose::default());
        assert_eq!(s.emotion, None);
    }

    #[test]
    fn replacing_script_re_anchors() {
        let script1 = make_script(vec![Keyframe {
            at_ms: 0,
            pan_deg: Some(10.0),
            ..Keyframe::default()
        }]);
        let script2 = make_script(vec![Keyframe {
            at_ms: 0,
            pan_deg: Some(-10.0),
            ..Keyframe::default()
        }]);
        let mut player = DancePlayer::new();
        player.load_script(script1, Instant::from_millis(0));

        let mut entity = entity_at(0);
        player.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 10.0);

        // Replace script. Re-anchor at the same instant; pose should
        // reflect the new script's first keyframe.
        player.load_script(script2, Instant::from_millis(0));
        player.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, -10.0);
    }

    #[test]
    fn default_matches_new() {
        let from_default = <DancePlayer as Default>::default();
        let from_new = DancePlayer::new();
        // Pin every field — covers the diff-and-undo state too, not
        // just the script slot. A future #[derive(Default)] swap with
        // different field defaults would surface here.
        assert!(from_default.active.is_none());
        assert!(from_new.active.is_none());
        assert!((from_default.last_pan_deg - from_new.last_pan_deg).abs() < f32::EPSILON);
        assert!((from_default.last_tilt_deg - from_new.last_tilt_deg).abs() < f32::EPSILON);
        assert_eq!(from_default.held_emotion, from_new.held_emotion);
        assert_eq!(from_default.held_decorator, from_new.held_decorator);
        assert_eq!(from_default.held_led, from_new.held_led);
    }

    #[test]
    fn meta_declares_motion_phase_with_dance_writes() {
        let m = DancePlayer::new();
        let meta = m.meta();
        assert_eq!(meta.name, "DancePlayer");
        assert_eq!(meta.phase, Phase::Motion);
        assert_eq!(meta.priority, 40);
        // Full slice equality on both reads and writes — catches any
        // future drop or reorder.
        assert_eq!(meta.reads, &[Field::HeadPose, Field::DanceScript],);
        assert_eq!(
            meta.writes,
            &[
                Field::HeadPose,
                Field::Emotion,
                Field::Autonomy,
                Field::Decorator,
                Field::LedOverride,
                Field::DanceScript,
            ],
        );
    }
}
