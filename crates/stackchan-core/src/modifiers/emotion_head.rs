//! `EmotionHead`: translate [`Emotion`] into a head pan/tilt **bias**.
//!
//! Runs after [`IdleSway`](super::IdleSway) in the modifier pipeline and
//! adds its bias on top of whatever sway has already written — the same
//! layered-pose pattern foreshadowed in `IdleSway`'s doc comment. Sway
//! wanders the head around a *biased* center rather than fighting with
//! a second absolute-set source.
//!
//! Transition timing matches [`EmotionStyle`](super::EmotionStyle): 300 ms
//! linear ease so the face and head both finish their transition at the
//! same moment, giving the emotion change a single coherent feel rather
//! than two staggered animations.
//!
//! ## Per-emotion bias table
//!
//! | Emotion    | Pan (°) | Tilt (°) | Why |
//! |------------|---------|----------|-----|
//! | Neutral    |     0.0 |      0.0 | Rest pose. |
//! | Happy      |     0.0 |     +3.0 | Chin up, confident. |
//! | Sad        |     0.0 |     -4.0 | Head drops slightly. |
//! | Sleepy     |     0.0 |     -6.0 | Head droops forward. |
//! | Surprised  |     0.0 |     +2.0 | Small back-and-up recoil. |
//!
//! Pan bias is zero across the board — desk-toy emotions don't read as
//! left/right turns, and `IdleSway` already contributes the natural pan
//! variation. The tilt values are conservative; combined with
//! `IdleSway`'s ±2.5° tilt, worst-case tilt reaches ~8.5° — well inside
//! [`MAX_TILT_DEG`](crate::head::MAX_TILT_DEG) (30°).

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;
use crate::head::Pose;

/// Per-emotion head-bias target.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeadBias {
    /// Pan bias in degrees (positive = right, per `Pose` conventions).
    pan_deg: f32,
    /// Tilt bias in degrees (positive = nod up).
    tilt_deg: f32,
}

impl HeadBias {
    /// The zero bias (used for Neutral + as the initial "from" target).
    const ZERO: Self = Self {
        pan_deg: 0.0,
        tilt_deg: 0.0,
    };
}

/// Constant look-up of the head-bias target for every [`Emotion`] variant.
/// Kept as a plain `match` so adding an `Emotion` variant becomes a
/// compile error here.
const fn targets_for(emotion: Emotion) -> HeadBias {
    match emotion {
        Emotion::Neutral => HeadBias::ZERO,
        Emotion::Happy => HeadBias {
            pan_deg: 0.0,
            tilt_deg: 3.0,
        },
        Emotion::Sad => HeadBias {
            pan_deg: 0.0,
            tilt_deg: -4.0,
        },
        Emotion::Sleepy => HeadBias {
            pan_deg: 0.0,
            tilt_deg: -6.0,
        },
        Emotion::Surprised => HeadBias {
            pan_deg: 0.0,
            tilt_deg: 2.0,
        },
    }
}

/// A modifier that translates `avatar.emotion` into a head-pose bias and
/// adds it onto `avatar.head_pose`.
///
/// Mirrors [`EmotionStyle`](super::EmotionStyle): carries `from` / `to`
/// state slots and a transition start instant so transitions ease rather
/// than snap. The initial tick (before any emotion has been seen) snaps
/// straight to the current emotion's bias — there's nothing to ease from.
#[derive(Debug, Clone, Copy)]
pub struct EmotionHead {
    /// Duration of an emotion transition, in milliseconds.
    transition_ms: u64,
    /// Bias we're transitioning *from*. `None` on the first tick.
    from: Option<HeadBias>,
    /// Bias we're transitioning *to* — the most recently observed
    /// `avatar.emotion`.
    to: Option<HeadBias>,
    /// Which emotion the `to` target corresponds to; used to detect
    /// `avatar.emotion` changes.
    to_emotion: Option<Emotion>,
    /// Monotonic time the current transition began.
    transition_start: Option<Instant>,
    /// Bias applied on the previous tick; subtracted from
    /// `avatar.head_pose` before writing the new one so the bias
    /// contribution stays a delta rather than accumulating.
    last_applied: HeadBias,
}

impl EmotionHead {
    /// Default transition duration, matching [`super::EmotionStyle::TRANSITION_MS`].
    pub const TRANSITION_MS: u64 = 300;

    /// Construct with the default 300 ms linear transition.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_transition_ms(Self::TRANSITION_MS)
    }

    /// Construct with a custom transition duration.
    #[must_use]
    pub const fn with_transition_ms(transition_ms: u64) -> Self {
        Self {
            transition_ms,
            from: None,
            to: None,
            to_emotion: None,
            transition_start: None,
            last_applied: HeadBias::ZERO,
        }
    }

    /// Compute the currently-active bias, given the last-seen state and
    /// the current time.
    fn current_bias(&self, now: Instant) -> HeadBias {
        match (self.from, self.to, self.transition_start) {
            (Some(from), Some(to), Some(start)) => {
                let elapsed = now.saturating_duration_since(start);
                ease_bias(from, to, elapsed, self.transition_ms)
            }
            // No previous from -> snap to current target.
            (None, Some(to), _) => to,
            // Never seen an emotion -> no bias yet.
            _ => HeadBias::ZERO,
        }
    }
}

impl Default for EmotionHead {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for EmotionHead {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let observed = targets_for(avatar.emotion);

        // Detect an emotion change: snapshot the previous `to` as `from`,
        // install the new `to`, reset the transition clock. On the very
        // first observation `self.to` is still `None` — leave `self.from`
        // `None` too so `current_bias` takes the snap-to-target branch
        // instead of easing ZERO→target at t=0 (which returns ZERO).
        if self.to_emotion != Some(avatar.emotion) {
            self.from = self.to;
            self.to = Some(observed);
            self.to_emotion = Some(avatar.emotion);
            self.transition_start = Some(now);
        }

        let bias = self.current_bias(now);

        // Layered compose via diff-and-undo (matches `IdleSway` /
        // `Breath`): subtract our previous *applied* (post-clamp) bias
        // from the current pose to recover upstream, add the new bias
        // request, then clamp. Storing the effective contribution into
        // `last_applied` (rather than the intended `bias`) keeps the
        // next tick's "undo" honest under asymmetric tilt clamping.
        let upstream = Pose::new(
            avatar.head_pose.pan_deg - self.last_applied.pan_deg,
            avatar.head_pose.tilt_deg - self.last_applied.tilt_deg,
        );
        let combined = Pose::new(
            upstream.pan_deg + bias.pan_deg,
            upstream.tilt_deg + bias.tilt_deg,
        )
        .clamped();
        self.last_applied = HeadBias {
            pan_deg: combined.pan_deg - upstream.pan_deg,
            tilt_deg: combined.tilt_deg - upstream.tilt_deg,
        };
        avatar.head_pose = combined;
    }
}

/// Linearly ease each field of a [`HeadBias`] from `from` to `to` over
/// `duration_ms`. Returns `to` once the window has elapsed.
fn ease_bias(from: HeadBias, to: HeadBias, elapsed_ms: u64, duration_ms: u64) -> HeadBias {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return to;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "elapsed + duration are < 2^32, well under the f32 mantissa limit"
    )]
    let t = elapsed_ms as f32 / duration_ms as f32;
    HeadBias {
        pan_deg: lerp(from.pan_deg, to.pan_deg, t),
        tilt_deg: lerp(from.tilt_deg, to.tilt_deg, t),
    }
}

/// Linear interpolation `from` → `to` by fraction `t`.
///
/// `mul_add` would be more accurate but routes through an `fma`
/// intrinsic that needs libm on `no_std` — same tradeoff as in
/// [`IdleSway::unit_triangle`](super::IdleSway).
#[allow(
    clippy::suboptimal_flops,
    reason = "avoiding libm dep — precision is ample for ±MAX_*_DEG servo output"
)]
fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare bit-exact outputs of our own lerp math, \
              not results of accumulated FP arithmetic"
)]
#[allow(
    clippy::field_reassign_with_default,
    reason = "test setup reads better as `let mut a = Avatar::default(); a.emotion = …;` \
              than a struct-update expression that repeats every field by default"
)]
mod tests {
    use super::*;

    #[test]
    fn neutral_emotion_produces_zero_bias() {
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Neutral;
        avatar.head_pose = Pose::new(1.0, 2.0); // simulate IdleSway output
        let mut eh = EmotionHead::new();
        // First tick: snap, then hold.
        eh.update(&mut avatar, Instant::from_millis(0));
        // With no previous emotion, the modifier snaps to Neutral (zero bias).
        assert_eq!(avatar.head_pose.pan_deg, 1.0);
        assert_eq!(avatar.head_pose.tilt_deg, 2.0);
    }

    #[test]
    fn happy_adds_tilt_bias_after_ease_completes() {
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Happy;
        avatar.head_pose = Pose::new(0.0, 0.0);
        let mut eh = EmotionHead::new();

        // Past TRANSITION_MS: the full Happy bias is applied.
        eh.update(
            &mut avatar,
            Instant::from_millis(EmotionHead::TRANSITION_MS + 1),
        );
        assert_eq!(avatar.head_pose.tilt_deg, 3.0);
        assert_eq!(avatar.head_pose.pan_deg, 0.0);
    }

    #[test]
    fn transition_eases_halfway() {
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Neutral;
        let mut eh = EmotionHead::new();

        // Start at Neutral (bias 0).
        eh.update(&mut avatar, Instant::from_millis(0));

        // Switch to Happy; the modifier captures the 'from' (0) and
        // 'to' (+3 tilt) and starts a 300 ms transition.
        avatar.emotion = Emotion::Happy;
        avatar.head_pose = Pose::new(0.0, 0.0);
        eh.update(&mut avatar, Instant::from_millis(0));

        // 150 ms in = halfway. Reset base each tick to isolate the bias.
        avatar.head_pose = Pose::new(0.0, 0.0);
        eh.update(&mut avatar, Instant::from_millis(150));
        assert!(
            (avatar.head_pose.tilt_deg - 1.5).abs() < 0.01,
            "expected tilt ≈1.5°, got {}",
            avatar.head_pose.tilt_deg
        );
    }

    #[test]
    fn bias_is_additive_on_top_of_existing_pose() {
        // Use a positive-tilt emotion so the test exercises additive
        // composition without colliding with the asymmetric tilt clamp
        // (downward tilts get pinned to MIN_TILT_DEG = 0 — see below).
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Happy;
        let mut eh = EmotionHead::new();

        // Advance past transition so the full bias applies.
        avatar.head_pose = Pose::new(2.0, 1.5); // sway output
        eh.update(
            &mut avatar,
            Instant::from_millis(EmotionHead::TRANSITION_MS + 1),
        );
        // Happy tilt bias is +3 → final tilt = 1.5 + 3 = 4.5.
        assert_eq!(avatar.head_pose.pan_deg, 2.0);
        assert_eq!(avatar.head_pose.tilt_deg, 4.5);
    }

    #[test]
    fn negative_tilt_bias_clamps_to_min_when_combined() {
        // Sleepy's bias is -6°; with `MIN_TILT_DEG = 0` (chassis can't
        // tilt below horizontal) the combined pose pins to 0.
        use crate::head::MIN_TILT_DEG;
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Sleepy;
        let mut eh = EmotionHead::new();

        avatar.head_pose = Pose::new(0.0, -1.5);
        eh.update(
            &mut avatar,
            Instant::from_millis(EmotionHead::TRANSITION_MS + 1),
        );
        assert_eq!(avatar.head_pose.tilt_deg, MIN_TILT_DEG);
    }

    #[test]
    fn changing_emotion_mid_transition_re_anchors_from_current() {
        // Drive the pipeline naturally (no manual head_pose resets) and
        // track the tilt trajectory across the mid-transition flip. With
        // diff-and-undo composition, the interesting invariant is that
        // the eventual steady-state matches the destination emotion; at
        // no point does the bias double-count.
        //
        // Uses Surprised (+2°) and Happy (+3°) — both in-range positive
        // biases — so the asymmetric tilt clamp doesn't mask the
        // re-anchoring behaviour we're testing.
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Neutral;
        let mut eh = EmotionHead::new();
        eh.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.head_pose.tilt_deg, 0.0, "Neutral snaps to zero bias");

        // Flip to Surprised; midway through the transition the bias is ~+1.
        avatar.emotion = Emotion::Surprised;
        eh.update(&mut avatar, Instant::from_millis(0));
        eh.update(&mut avatar, Instant::from_millis(150));
        let mid_surprised = avatar.head_pose.tilt_deg;
        assert!(
            (mid_surprised - 1.0).abs() < 0.1,
            "mid-transition Surprised tilt should be ~+1, got {mid_surprised}"
        );

        // Flip to Happy mid-transition. After the new window elapses,
        // we land on Happy's +3.
        avatar.emotion = Emotion::Happy;
        eh.update(&mut avatar, Instant::from_millis(150));
        eh.update(
            &mut avatar,
            Instant::from_millis(150 + EmotionHead::TRANSITION_MS + 1),
        );
        assert_eq!(
            avatar.head_pose.tilt_deg, 3.0,
            "should fully land on Happy's +3° tilt after the transition elapses"
        );
    }

    #[test]
    fn bias_does_not_accumulate_across_ticks() {
        // Regression for the diff-and-undo refactor: previously
        // EmotionHead added bias absolutely, which relied on IdleSway
        // clobbering head_pose first. Now that IdleSway also contributes
        // additively, EmotionHead must subtract the previous tick's bias
        // before adding the new one — otherwise a steady-state emotion
        // would see its bias compound each tick.
        //
        // Uses Happy (+3°) — a positive in-range bias — so the
        // asymmetric tilt clamp can't disguise an accumulation bug
        // by saturating both the buggy and correct cases at MIN_TILT_DEG.
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Happy; // +3° tilt bias at steady state
        let mut eh = EmotionHead::new();

        // Drive past the transition, then many more ticks. The tilt must
        // stay at +3, not drift to +6, +9, ...
        for i in 0..=50 {
            eh.update(
                &mut avatar,
                Instant::from_millis(EmotionHead::TRANSITION_MS + i * 33),
            );
        }
        assert_eq!(avatar.head_pose.tilt_deg, 3.0);
    }

    #[test]
    fn clamp_engages_when_combined_exceeds_safe_range() {
        use crate::head::{MAX_TILT_DEG, MIN_TILT_DEG};

        // Upper clamp: hostile upstream + Happy (+3) push past MAX.
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Happy;
        avatar.head_pose = Pose::new(0.0, 29.0);
        let mut eh = EmotionHead::new();
        eh.update(
            &mut avatar,
            Instant::from_millis(EmotionHead::TRANSITION_MS + 1),
        );
        assert_eq!(avatar.head_pose.tilt_deg, MAX_TILT_DEG);

        // Lower clamp: hostile upstream + Sleepy (-6) push below MIN.
        // Asymmetric tilt range — MIN_TILT_DEG is 0, not -MAX_TILT_DEG.
        let mut avatar = Avatar::default();
        avatar.emotion = Emotion::Sleepy;
        avatar.head_pose = Pose::new(0.0, -29.0);
        let mut eh = EmotionHead::new();
        eh.update(
            &mut avatar,
            Instant::from_millis(EmotionHead::TRANSITION_MS + 1),
        );
        assert_eq!(avatar.head_pose.tilt_deg, MIN_TILT_DEG);
    }

    #[test]
    fn zero_duration_snaps_without_easing() {
        let b = ease_bias(
            HeadBias::ZERO,
            HeadBias {
                pan_deg: 10.0,
                tilt_deg: -5.0,
            },
            50,
            0,
        );
        assert_eq!(b.pan_deg, 10.0);
        assert_eq!(b.tilt_deg, -5.0);
    }
}
