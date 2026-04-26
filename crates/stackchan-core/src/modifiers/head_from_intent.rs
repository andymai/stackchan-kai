//! `HeadFromIntent`: motion modifier that fires a brief head recoil on
//! the entry edge into [`Intent::Startled`].
//!
//! Driven by [`super::IntentFromLoud`] (which sets the intent on a
//! loud-threshold rising edge). The recoil is fixed-duration —
//! [`STARTLE_HEAD_TOTAL_MS`] — so even if `Intent::Startled` holds
//! for the full [`super::STARTLE_HOLD_MS`], the head returns to its
//! upstream pose well before the intent clears.
//!
//! ## Shape
//!
//! Asymmetric triangle: fast attack to peak over
//! [`STARTLE_HEAD_ATTACK_MS`], slower decay to zero over
//! [`STARTLE_HEAD_DECAY_MS`]. Mimics the orienting reflex — a quick
//! "what was that?!" posture that settles.
//!
//! Peak: small upward tilt ([`STARTLE_HEAD_TILT_DEG`]) plus a slight
//! pan jerk ([`STARTLE_HEAD_PAN_DEG`]). Pan direction is fixed (no
//! sound-source localisation on a single mic); the recoil reads as
//! "alert, head slightly turned" rather than "tracking."
//!
//! ## Composition
//!
//! Runs after [`super::IdleHeadDrift`] (priority 0), [`super::HeadFromEmotion`]
//! (priority 10), and [`super::HeadFromAttention`] (priority 20) within
//! [`Phase::Motion`], so its bias rides on top of all three. Same
//! diff-and-undo pattern as the other [`Phase::Motion`] modifiers:
//! subtract the previous applied contribution before adding the new
//! one, store the post-clamp delta so asymmetric clamping doesn't
//! accumulate.

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::head::Pose;
use crate::mind::Intent;
use crate::modifier::Modifier;

/// Peak upward tilt added during the recoil, in degrees.
///
/// `+6°` reads as "alert, head up" without overpowering the other
/// motion modifiers (combined with `IdleHeadDrift`'s up-to-±3° tilt
/// and `HeadFromEmotion`'s up-to-+3° plus `HeadFromAttention`'s +8°
/// upper bound, the worst-case stays inside
/// [`MAX_TILT_DEG`](crate::head::MAX_TILT_DEG)).
pub const STARTLE_HEAD_TILT_DEG: f32 = 6.0;

/// Peak pan offset during the recoil, in degrees.
///
/// `+5°` (right-ish jerk) is small enough that combined with
/// `IdleHeadDrift` and `HeadFromEmotion`'s pan contributions, the result stays
/// inside [`MAX_PAN_DEG`](crate::head::MAX_PAN_DEG).
pub const STARTLE_HEAD_PAN_DEG: f32 = 5.0;

/// Attack time: how long to ramp from zero up to peak, in ms.
///
/// `50 ms` is at the edge of perceptible motion onset — fast enough
/// to read as a startle, slow enough that the servo can follow.
pub const STARTLE_HEAD_ATTACK_MS: u64 = 50;

/// Decay time: how long to ramp from peak back to zero, in ms.
///
/// `350 ms` gives an asymmetric attack/decay shape — the longer
/// settle reads as the avatar relaxing after the moment of alarm.
pub const STARTLE_HEAD_DECAY_MS: u64 = 350;

/// Total recoil duration. After `attack + decay` ms past the entry
/// edge the modifier contributes zero, even if `Intent::Startled`
/// is still held.
pub const STARTLE_HEAD_TOTAL_MS: u64 = STARTLE_HEAD_ATTACK_MS + STARTLE_HEAD_DECAY_MS;

/// Modifier that translates the entry edge into `Intent::Startled`
/// into a brief asymmetric recoil on `motor.head_pose`.
#[derive(Debug, Clone, Copy)]
pub struct HeadFromIntent {
    /// Pan contribution as actually applied last tick (post-clamp).
    /// Diff-and-undo bookkeeping — see `HeadFromAttention::last_tilt_deg`.
    last_pan_deg: f32,
    /// Tilt contribution as actually applied last tick (post-clamp).
    last_tilt_deg: f32,
    /// Was the previous tick `Intent::Startled`? Drives edge
    /// detection so we anchor on the entry edge.
    was_hearing_loud: bool,
    /// Instant the most recent recoil began. `None` outside an active
    /// recoil window.
    started_at: Option<Instant>,
}

impl HeadFromIntent {
    /// Construct an idle modifier with no in-flight recoil.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_pan_deg: 0.0,
            last_tilt_deg: 0.0,
            was_hearing_loud: false,
            started_at: None,
        }
    }
}

impl Default for HeadFromIntent {
    fn default() -> Self {
        Self::new()
    }
}

/// Asymmetric triangle envelope.
///
/// Returns the unit-amplitude bias `0.0..=1.0` at `elapsed` ms past
/// the recoil start: linear ramp up over `attack_ms`, linear ramp
/// down over `decay_ms`, zero outside the window.
#[allow(
    clippy::cast_precision_loss,
    reason = "elapsed and the windows are bounded by STARTLE_HEAD_TOTAL_MS — well under 2^24"
)]
fn envelope(elapsed_ms: u64, attack_ms: u64, decay_ms: u64) -> f32 {
    if elapsed_ms < attack_ms {
        if attack_ms == 0 {
            return 1.0;
        }
        return elapsed_ms as f32 / attack_ms as f32;
    }
    let decay_elapsed = elapsed_ms - attack_ms;
    if decay_elapsed >= decay_ms {
        return 0.0;
    }
    if decay_ms == 0 {
        return 0.0;
    }
    1.0 - (decay_elapsed as f32 / decay_ms as f32)
}

impl Modifier for HeadFromIntent {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "HeadFromIntent",
            description: "On entry to Intent::Startled, applies a brief asymmetric recoil \
                          (fast attack, slower decay) to motor.head_pose pan + tilt. Total \
                          duration STARTLE_HEAD_TOTAL_MS regardless of how long the intent \
                          holds. Composes additively after IdleHeadDrift / HeadFromEmotion / HeadFromAttention \
                          via diff-and-undo.",
            phase: Phase::Motion,
            priority: 30,
            reads: &[Field::Intent, Field::HeadPose],
            writes: &[Field::HeadPose],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let hearing_loud = matches!(entity.mind.intent, Intent::Startled);

        // Anchor a fresh recoil on the entry edge. Re-entry after a
        // completed recoil restarts the envelope; re-entry while
        // already mid-recoil is suppressed (no double-anchor).
        if hearing_loud && !self.was_hearing_loud {
            self.started_at = Some(now);
        }
        self.was_hearing_loud = hearing_loud;

        // Compute the current bias amplitude. Clear the anchor only
        // once the envelope has fully completed (past attack + decay)
        // — clearing on `amplitude == 0` would also fire on the
        // entry tick when elapsed = 0 and kill the recoil before it
        // starts.
        let amplitude = match self.started_at {
            Some(start) => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= STARTLE_HEAD_TOTAL_MS {
                    self.started_at = None;
                    0.0
                } else {
                    envelope(elapsed, STARTLE_HEAD_ATTACK_MS, STARTLE_HEAD_DECAY_MS)
                }
            }
            None => 0.0,
        };

        let target_pan = STARTLE_HEAD_PAN_DEG * amplitude;
        let target_tilt = STARTLE_HEAD_TILT_DEG * amplitude;

        // Diff-and-undo composition. Subtract our previous applied
        // contribution to recover upstream, add the new one, clamp,
        // store the post-clamp effective delta.
        let upstream_pan = entity.motor.head_pose.pan_deg - self.last_pan_deg;
        let upstream_tilt = entity.motor.head_pose.tilt_deg - self.last_tilt_deg;
        let combined = Pose::new(upstream_pan + target_pan, upstream_tilt + target_tilt).clamped();
        self.last_pan_deg = combined.pan_deg - upstream_pan;
        self.last_tilt_deg = combined.tilt_deg - upstream_tilt;
        entity.motor.head_pose = combined;
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact outputs of our own envelope math"
)]
mod tests {
    use super::*;
    use crate::Entity;

    fn entity_at(now_ms: u64, intent: Intent) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e.mind.intent = intent;
        e
    }

    #[test]
    fn no_intent_leaves_pose_alone() {
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Idle);
        entity.motor.head_pose = Pose::new(2.0, 1.0);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose, Pose::new(2.0, 1.0));
    }

    #[test]
    fn entry_edge_anchors_recoil() {
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Idle);
        m.update(&mut entity);

        // Entry edge: amplitude = 0 on the entry tick (elapsed = 0).
        entity.mind.intent = Intent::Startled;
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);

        // After full attack window: amplitude = 1, peak bias.
        entity.tick.now = Instant::from_millis(STARTLE_HEAD_ATTACK_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, STARTLE_HEAD_PAN_DEG);
        assert_eq!(entity.motor.head_pose.tilt_deg, STARTLE_HEAD_TILT_DEG);
    }

    #[test]
    fn decay_returns_to_zero() {
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Startled);
        m.update(&mut entity);

        // Past total duration → no contribution.
        entity.tick.now = Instant::from_millis(STARTLE_HEAD_TOTAL_MS + 1);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);
    }

    #[test]
    fn recoil_completes_even_if_intent_holds() {
        // Intent::Startled may hold for the full STARTLE_HOLD_MS
        // (1500ms) but the head recoil only lasts STARTLE_HEAD_TOTAL_MS
        // (400ms). Verify the head returns to upstream well before
        // the intent clears.
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Startled);
        m.update(&mut entity);

        // 1000 ms in — well past the recoil window, intent still held.
        entity.tick.now = Instant::from_millis(1_000);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);
        assert_eq!(entity.motor.head_pose.tilt_deg, 0.0);
    }

    #[test]
    fn re_entry_after_completion_re_anchors() {
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Startled);
        m.update(&mut entity);

        // Wait out full recoil.
        entity.tick.now = Instant::from_millis(STARTLE_HEAD_TOTAL_MS + 100);
        m.update(&mut entity);

        // Drop intent.
        entity.mind.intent = Intent::Idle;
        entity.tick.now = Instant::from_millis(STARTLE_HEAD_TOTAL_MS + 200);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, 0.0);

        // Re-enter. Fresh recoil starts; peak after attack window.
        entity.mind.intent = Intent::Startled;
        let restart = STARTLE_HEAD_TOTAL_MS + 300;
        entity.tick.now = Instant::from_millis(restart);
        m.update(&mut entity);
        entity.tick.now = Instant::from_millis(restart + STARTLE_HEAD_ATTACK_MS);
        m.update(&mut entity);
        assert_eq!(entity.motor.head_pose.pan_deg, STARTLE_HEAD_PAN_DEG);
        assert_eq!(entity.motor.head_pose.tilt_deg, STARTLE_HEAD_TILT_DEG);
    }

    #[test]
    fn rides_on_top_of_upstream_pose_at_peak() {
        // With an upstream pose of (-2°, 5°) preserved through the
        // modifier's diff-and-undo, the recoil contribution at peak
        // should equal STARTLE_HEAD_PAN_DEG / TILT_DEG (within FP
        // tolerance), and the absolute pose stays inside clamps.
        let mut m = HeadFromIntent::new();
        let mut entity = entity_at(0, Intent::Startled);
        let upstream_pan = -2.0;
        let upstream_tilt = 5.0;
        entity.motor.head_pose = Pose::new(upstream_pan, upstream_tilt);

        // Entry tick: amplitude = 0, pose = upstream.
        m.update(&mut entity);
        // Peak tick: amplitude = 1.
        entity.tick.now = Instant::from_millis(STARTLE_HEAD_ATTACK_MS);
        m.update(&mut entity);

        let pan_contribution = entity.motor.head_pose.pan_deg - upstream_pan;
        let tilt_contribution = entity.motor.head_pose.tilt_deg - upstream_tilt;
        assert!(
            (pan_contribution - STARTLE_HEAD_PAN_DEG).abs() < 0.01,
            "pan contribution {pan_contribution} != peak {STARTLE_HEAD_PAN_DEG}",
        );
        assert!(
            (tilt_contribution - STARTLE_HEAD_TILT_DEG).abs() < 0.01,
            "tilt contribution {tilt_contribution} != peak {STARTLE_HEAD_TILT_DEG}",
        );
    }

    #[test]
    fn envelope_attack_is_linear_to_peak() {
        // Spot-check the envelope at attack/2: should be 0.5.
        let mid = envelope(
            STARTLE_HEAD_ATTACK_MS / 2,
            STARTLE_HEAD_ATTACK_MS,
            STARTLE_HEAD_DECAY_MS,
        );
        assert!((mid - 0.5).abs() < 0.01);
    }

    #[test]
    fn envelope_decay_is_linear_from_peak() {
        // Spot-check at peak + decay/2: should be 0.5.
        let mid = envelope(
            STARTLE_HEAD_ATTACK_MS + STARTLE_HEAD_DECAY_MS / 2,
            STARTLE_HEAD_ATTACK_MS,
            STARTLE_HEAD_DECAY_MS,
        );
        assert!((mid - 0.5).abs() < 0.01);
    }
}
