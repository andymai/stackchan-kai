//! `DormancyFromActivity`: cognition-phase modifier that flips
//! [`crate::mind::Dormancy`] between [`Dormancy::Awake`] and
//! [`Dormancy::Asleep`] based on how recently any activity-bearing
//! mind field changed away from its default.
//!
//! ## Why
//!
//! The avatar's idle background motion ([`crate::modifiers::IdleSway`]'s
//! triangle wave on `motor.head_pose`) keeps the `SCServo` head
//! continuously slewing even when no one is in the room — audible
//! from the next room and a small but real battery drain. Gating the
//! sway when nothing has happened for a while silences the head
//! without changing the avatar's behaviour while a person is present.
//!
//! ## Activity signals
//!
//! Any of the following counts as activity, resetting the timeout:
//!
//! - `mind.intent != Intent::Idle` — body touch, IR remote, loud
//!   audio, pickup, shake, sustained voice all flip intent.
//! - `mind.attention != Attention::None` — sustained voice via
//!   [`crate::skills::Listening`], camera motion via
//!   [`crate::modifiers::AttentionFromTracking`].
//! - `mind.engagement != Engagement::Idle` — the cascade is
//!   reporting a face (Locking / Locked / Releasing).
//!
//! These three together cover every modality the firmware exposes
//! today (audio, vision, touch, remote, IMU). A future modality
//! (e.g. an explicit "user spoke their name" event) should land on
//! one of these three so it wakes the avatar without a code change
//! here.
//!
//! ## Phase + priority
//!
//! Runs in [`Phase::Cognition`] at priority `10`, after
//! [`crate::modifiers::AttentionFromTracking`] (priority `0`) so
//! `engagement` and `attention` reflect *this* tick's observation
//! before we read them. Writing happens before [`Phase::Expression`]
//! and [`Phase::Motion`] so [`crate::modifiers::IdleSway`] sees the
//! fresh dormancy value the same tick.
//!
//! ## Boot
//!
//! Boot is treated as activity — `last_active_at` anchors on the
//! first tick — so the avatar wakes on power-on and the dormant
//! transition fires `DORMANCY_TIMEOUT_MS` after boot if nothing
//! engages it in that window.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::mind::{Attention, Dormancy, Engagement, Intent};
use crate::modifier::Modifier;

/// How long, in ms, all three activity signals must stay at their
/// defaults before the modifier transitions to [`Dormancy::Asleep`].
///
/// `30_000` ms reads as "the room has been quiet for half a minute"
/// — long enough that brief silences during a real interaction
/// don't trip dormancy, short enough that walking away from the desk
/// silences the head before it becomes background-noise irritating.
pub const DORMANCY_TIMEOUT_MS: u64 = 30_000;

/// Modifier that drives [`crate::mind::Dormancy`] from the activity
/// signals on `entity.mind`.
#[derive(Debug, Clone, Copy)]
pub struct DormancyFromActivity {
    /// Per-instance timeout. Defaults to [`DORMANCY_TIMEOUT_MS`];
    /// override via [`Self::with_timeout`] for tests or unusual
    /// deployments.
    pub timeout_ms: u64,
    /// Wall-clock instant of the most recent active tick. `None`
    /// before the first tick; the modifier seeds this on its first
    /// invocation so boot itself counts as activity.
    last_active_at: Option<Instant>,
}

impl DormancyFromActivity {
    /// Construct with the default [`DORMANCY_TIMEOUT_MS`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            timeout_ms: DORMANCY_TIMEOUT_MS,
            last_active_at: None,
        }
    }

    /// Construct with a custom timeout, in ms.
    #[must_use]
    pub const fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            timeout_ms,
            last_active_at: None,
        }
    }
}

impl Default for DormancyFromActivity {
    fn default() -> Self {
        Self::new()
    }
}

/// `true` when any activity-bearing mind field is non-default.
const fn is_active(entity: &Entity) -> bool {
    !matches!(entity.mind.intent, Intent::Idle)
        || !matches!(entity.mind.attention, Attention::None)
        || !matches!(entity.mind.engagement, Engagement::Idle)
}

impl Modifier for DormancyFromActivity {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "DormancyFromActivity",
            description: "Watches mind.{intent, attention, engagement}; sets \
                          mind.dormancy = Awake on any non-default activity, \
                          and Asleep after DORMANCY_TIMEOUT_MS of full quiet. \
                          Lets IdleSway gate its triangle-wave contribution so \
                          the head servos stay still when nothing's happening \
                          in the room.",
            phase: Phase::Cognition,
            // After AttentionFromTracking (priority 0) so engagement /
            // attention reflect this tick's observation before we read.
            priority: 10,
            reads: &[
                Field::Intent,
                Field::Attention,
                Field::Engagement,
                Field::Dormancy,
            ],
            writes: &[Field::Dormancy],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;

        // Seed on first tick so boot counts as activity. Without
        // this, `last_active_at` stays `None` forever in a totally
        // quiet room and the dormant transition fires immediately
        // (since `now - 0` is large).
        if self.last_active_at.is_none() {
            self.last_active_at = Some(now);
        }

        if is_active(entity) {
            self.last_active_at = Some(now);
            entity.mind.dormancy = Dormancy::Awake;
            return;
        }

        // Quiet tick. Compare elapsed-since-last-active against the
        // timeout; transition to Asleep on the crossing.
        let last = self.last_active_at.unwrap_or(now);
        let elapsed = now.saturating_duration_since(last);
        if elapsed >= self.timeout_ms && !entity.mind.dormancy.is_asleep() {
            entity.mind.dormancy = Dormancy::Asleep { since: now };
        }
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

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    #[test]
    fn fresh_modifier_keeps_avatar_awake_until_timeout() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);
        assert_eq!(entity.mind.dormancy, Dormancy::Awake);
        // Just shy of the timeout: still awake.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS - 1);
        m.update(&mut entity);
        assert_eq!(entity.mind.dormancy, Dormancy::Awake);
    }

    #[test]
    fn quiet_past_timeout_flips_to_asleep() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS);
        m.update(&mut entity);
        match entity.mind.dormancy {
            Dormancy::Asleep { since } => {
                assert_eq!(since, Instant::from_millis(DORMANCY_TIMEOUT_MS));
            }
            other => panic!("expected Asleep, got {other:?}"),
        }
    }

    #[test]
    fn intent_activity_keeps_avatar_awake() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        // Sustained activity past the timeout: must stay Awake.
        for t in (0..DORMANCY_TIMEOUT_MS + 1_000).step_by(100) {
            entity.tick.now = Instant::from_millis(t);
            entity.mind.intent = Intent::Petted;
            m.update(&mut entity);
            assert_eq!(entity.mind.dormancy, Dormancy::Awake);
        }
    }

    #[test]
    fn engagement_activity_keeps_avatar_awake() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        for t in (0..DORMANCY_TIMEOUT_MS + 1_000).step_by(100) {
            entity.tick.now = Instant::from_millis(t);
            entity.mind.engagement = Engagement::Locking { hits: 1 };
            m.update(&mut entity);
            assert_eq!(entity.mind.dormancy, Dormancy::Awake);
        }
    }

    #[test]
    fn attention_activity_keeps_avatar_awake() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        for t in (0..DORMANCY_TIMEOUT_MS + 1_000).step_by(100) {
            entity.tick.now = Instant::from_millis(t);
            entity.mind.attention = Attention::Listening {
                since: Instant::from_millis(t),
            };
            m.update(&mut entity);
            assert_eq!(entity.mind.dormancy, Dormancy::Awake);
        }
    }

    #[test]
    fn activity_after_asleep_wakes_immediately() {
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        // Sleep.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS);
        m.update(&mut entity);
        assert!(entity.mind.dormancy.is_asleep());

        // Activity arrives one tick later: wake immediately.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS + 33);
        entity.mind.intent = Intent::Startled;
        m.update(&mut entity);
        assert_eq!(entity.mind.dormancy, Dormancy::Awake);
    }

    #[test]
    fn brief_activity_then_quiet_resets_timeout() {
        // Activity → quiet for slightly less than the timeout →
        // brief activity → quiet for the full timeout from the
        // brief-activity point. Must NOT fall asleep at the original
        // timeout; only after the full timeout from the most recent
        // activity.
        let mut m = DormancyFromActivity::new();
        let mut entity = at(0);
        m.update(&mut entity);

        // Activity at tick 0; clear at tick 100.
        entity.mind.intent = Intent::Petted;
        m.update(&mut entity);

        entity.tick.now = Instant::from_millis(100);
        entity.mind.intent = Intent::Idle;
        m.update(&mut entity);

        // Wait until just shy of the original timeout.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS - 100);
        m.update(&mut entity);
        assert_eq!(entity.mind.dormancy, Dormancy::Awake);

        // Brief activity bump.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS - 50);
        entity.mind.intent = Intent::Petted;
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS);
        entity.mind.intent = Intent::Idle;
        m.update(&mut entity);
        // At the original timeout, the second-activity reset means
        // we're still awake.
        assert_eq!(entity.mind.dormancy, Dormancy::Awake);

        // Now wait the full timeout from the second activity.
        entity.tick.now = Instant::from_millis(DORMANCY_TIMEOUT_MS - 50 + DORMANCY_TIMEOUT_MS);
        m.update(&mut entity);
        assert!(entity.mind.dormancy.is_asleep());
    }

    #[test]
    fn custom_timeout_overrides_default() {
        let mut m = DormancyFromActivity::with_timeout(1_000);
        let mut entity = at(0);
        m.update(&mut entity);
        // Default timeout would NOT fire here, but our custom 1 s does.
        entity.tick.now = Instant::from_millis(1_000);
        m.update(&mut entity);
        assert!(entity.mind.dormancy.is_asleep());
    }
}
