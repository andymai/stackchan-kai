//! `StyleFromEmotion`: translate [`Emotion`] into the entity's style fields.
//!
//! This modifier is the single source of truth for how a given emotion
//! *looks*. It writes absolute target values to the entity's style fields
//! (`eye_curve`, `mouth_curve`, `cheek_blush`, `eye_scale`,
//! `blink_rate_scale`, `breath_depth_scale`) plus both eyes'
//! `open_weight`. The renderer and `Blink`/`Breath` read those fields
//! without knowing about `Emotion` itself.
//!
//! Transitions are linearly eased over [`StyleFromEmotion::TRANSITION_MS`]
//! so an emotion flip doesn't snap the face. The previous target is
//! captured on every emotion change; interpolation runs per-field.
//!
//! Modifier order matters: put this **before** `Blink`/`Breath` in the
//! tick so they see the freshly-eased scale values. It is safe to run
//! it after `IdleDrift` (which touches centers, not style fields).
//!
//! [`Emotion`]: crate::Emotion

use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::face::SCALE_DEFAULT;
use crate::modifier::Modifier;

/// Per-emotion style target. Every field an emotion can influence lives
/// here; the per-emotion table in [`targets_for`] is the authoritative
/// mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StyleTarget {
    /// Target `entity.face.style.eye_curve` (-100..=100).
    eye_curve: i8,
    /// Target `entity.face.style.mouth_curve` (-100..=100).
    mouth_curve: i8,
    /// Target `entity.face.style.cheek_blush` (0..=255).
    cheek_blush: u8,
    /// Target `entity.face.style.eye_scale` (0..=255, 128 = baseline).
    eye_scale: u8,
    /// Target `entity.face.style.blink_rate_scale` (0..=255, 128 = baseline, 0 = suppressed).
    blink_rate_scale: u8,
    /// Target `entity.face.style.breath_depth_scale` (0..=255, 128 = baseline).
    breath_depth_scale: u8,
    /// Target `Eye::open_weight` applied to both eyes (0..=100).
    open_weight: u8,
    /// Target `Mouth::weight` (0..=100). Only set when the emotion wants
    /// the mouth open (Surprised); other emotions leave this at 0 and
    /// express the mouth via `mouth_curve`.
    mouth_weight: u8,
}

/// Constant look-up of the style target for every [`Emotion`] variant.
/// Kept as a plain `match` (rather than a table) so adding a variant to
/// `Emotion` surfaces as a compile error here.
#[allow(
    clippy::too_many_lines,
    reason = "lookup table — splitting into per-emotion consts would shift \
              the line count without simplifying the surface, and the inline \
              comments next to each row are the load-bearing rationale"
)]
const fn targets_for(emotion: Emotion) -> StyleTarget {
    match emotion {
        Emotion::Neutral => StyleTarget {
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: SCALE_DEFAULT,
            breath_depth_scale: SCALE_DEFAULT,
            open_weight: 100,
            mouth_weight: 0,
        },
        Emotion::Happy => StyleTarget {
            // Upward eye arc + smile curve + light blush.
            eye_curve: 70,
            mouth_curve: 80,
            cheek_blush: 160,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: SCALE_DEFAULT,
            breath_depth_scale: SCALE_DEFAULT,
            open_weight: 100,
            mouth_weight: 0,
        },
        Emotion::Sad => StyleTarget {
            // Downward eye arc + frown + slow, deep breath.
            eye_curve: -55,
            mouth_curve: -70,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: 96,
            breath_depth_scale: 170,
            open_weight: 90,
            mouth_weight: 0,
        },
        Emotion::Sleepy => StyleTarget {
            // Half-closed droopy lids, very slow blinks, deep slow breath.
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: 48,
            breath_depth_scale: 200,
            open_weight: 55,
            mouth_weight: 0,
        },
        Emotion::Surprised => StyleTarget {
            // Wide-open eyes (no curve), held (no blinks), shallow breath,
            // open round mouth.
            eye_curve: 0,
            mouth_curve: 0,
            cheek_blush: 0,
            eye_scale: 170,
            blink_rate_scale: 0,
            breath_depth_scale: 64,
            open_weight: 100,
            mouth_weight: 100,
        },
        Emotion::Angry => StyleTarget {
            // Narrowed stern eyes, slight frown, flushed cheeks, deeper
            // breath. Less drooping than Sad; the frown is firmer.
            eye_curve: -25,
            mouth_curve: -50,
            cheek_blush: 80,
            eye_scale: 96,
            blink_rate_scale: 96,
            breath_depth_scale: 180,
            open_weight: 80,
            mouth_weight: 0,
        },
        Emotion::Doubt => StyleTarget {
            // Skeptical-positive: slight downward eye arc with a faint
            // smile that reads as "I'm not buying it but I'm amused."
            // Distinguished from `Confused` by the +mouth_curve sign.
            eye_curve: -15,
            mouth_curve: 20,
            cheek_blush: 0,
            eye_scale: 110,
            blink_rate_scale: 80,
            breath_depth_scale: 110,
            open_weight: 80,
            mouth_weight: 0,
        },
        Emotion::Boring => StyleTarget {
            // Disinterested: half-lidded eyes, slow blink, slow deep
            // breath, very faint frown. Distinguished from `Sleepy` by a
            // less-droopy lid and a non-zero mouth_curve.
            eye_curve: 0,
            mouth_curve: -15,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: 56,
            breath_depth_scale: 150,
            open_weight: 65,
            mouth_weight: 0,
        },
        Emotion::Hi => StyleTarget {
            // Outgoing greeting. Wider eyes than `Happy`, faster blink,
            // bigger smile arc — reads as a wave-hello.
            eye_curve: 50,
            mouth_curve: 70,
            cheek_blush: 100,
            eye_scale: 140,
            blink_rate_scale: 160,
            breath_depth_scale: 110,
            open_weight: 100,
            mouth_weight: 0,
        },
        Emotion::Loved => StyleTarget {
            // Smitten: full upward eye arc + heaviest blush of the
            // catalogue. The decorator layer paints heart overlays on
            // top in PR 1b; the base style stands alone here.
            eye_curve: 90,
            mouth_curve: 50,
            cheek_blush: 220,
            eye_scale: 110,
            blink_rate_scale: 140,
            breath_depth_scale: 110,
            open_weight: 100,
            mouth_weight: 0,
        },
        Emotion::Curious => StyleTarget {
            // Investigative: wider-than-baseline eyes (smaller than
            // `Surprised`), faint smile, light blush. The breath / blink
            // hold near baseline so the wide-eye reads as attention,
            // not startle.
            eye_curve: 30,
            mouth_curve: 25,
            cheek_blush: 40,
            eye_scale: 150,
            blink_rate_scale: 110,
            breath_depth_scale: SCALE_DEFAULT,
            open_weight: 100,
            mouth_weight: 0,
        },
        Emotion::Confused => StyleTarget {
            // Unsettled-negative: matching downward curves on eyes and
            // mouth + elevated blink rate (reads as eye-flutter).
            // Mirrors `Doubt` along valence: same uncertainty, opposite
            // mouth sign.
            eye_curve: -20,
            mouth_curve: -20,
            cheek_blush: 0,
            eye_scale: SCALE_DEFAULT,
            blink_rate_scale: 160,
            breath_depth_scale: SCALE_DEFAULT,
            open_weight: 95,
            mouth_weight: 0,
        },
        Emotion::Mad => StyleTarget {
            // Hotter than `Angry`: deeper frown, heavier blush, slower
            // huffing breath, narrower squint. The default
            // `EmotionCycle` doesn't visit this — it's reactive only.
            eye_curve: -55,
            mouth_curve: -85,
            cheek_blush: 120,
            eye_scale: 100,
            blink_rate_scale: 70,
            breath_depth_scale: 200,
            open_weight: 85,
            mouth_weight: 0,
        },
    }
}

/// Linearly ease an `i32` from `from` to `to` over `duration_ms`, given
/// `elapsed_ms` since the transition started. Returns `to` once
/// `elapsed_ms >= duration_ms`.
fn ease(from: i32, to: i32, elapsed_ms: u64, duration_ms: u64) -> i32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return to;
    }
    let span = to - from;
    // Intermediate math in i64 so `span * elapsed` can't overflow before
    // the `/duration` divides it back down.
    let progress = i64::from(span) * i64::try_from(elapsed_ms).unwrap_or(i64::MAX)
        / i64::try_from(duration_ms).unwrap_or(1);
    #[allow(clippy::cast_possible_truncation)]
    let delta = progress as i32;
    from + delta
}

/// Saturating clamp of an eased `i32` into `[min, max]`, casting to `u8`.
/// Shared between the `u8` style fields (range `0..=255`) and the
/// `weight` fields (range `0..=100`); the caller picks the bounds.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn clamp_to_u8(v: i32, min: u8, max: u8) -> u8 {
    let min = min as i32;
    let max = max as i32;
    if v <= min {
        min as u8
    } else if v >= max {
        max as u8
    } else {
        v as u8
    }
}

/// Saturating clamp of an eased `i32` into `i8::MIN..=i8::MAX`.
#[allow(clippy::cast_possible_truncation)]
const fn clamp_i8(v: i32) -> i8 {
    if v <= i8::MIN as i32 {
        i8::MIN
    } else if v >= i8::MAX as i32 {
        i8::MAX
    } else {
        v as i8
    }
}

/// A modifier that translates `entity.mind.affect.emotion` into the style fields,
/// linearly easing between emotions so transitions feel alive.
///
/// Carries two state slots — the last-seen emotion and the start of the
/// current transition — so it is idempotent across multiple `update` calls
/// at the same time.
#[derive(Debug, Clone, Copy)]
pub struct StyleFromEmotion {
    /// Duration of an emotion transition, in milliseconds.
    transition_ms: u64,
    /// Target state we're transitioning *from*. `None` on the very first
    /// tick (we snap instantly).
    from: Option<StyleTarget>,
    /// Target state we're transitioning *to* — the most recently observed
    /// `entity.mind.affect.emotion`.
    to: Option<StyleTarget>,
    /// Which emotion the `to` target corresponds to; used to detect
    /// `entity.mind.affect.emotion` changes.
    to_emotion: Option<Emotion>,
    /// Monotonic time the current transition began.
    transition_start: Option<Instant>,
}

impl StyleFromEmotion {
    /// Default transition duration, in milliseconds.
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
        }
    }

    /// Apply a fully-resolved [`StyleTarget`] to `entity`. Split out so
    /// the "snap-on-first-tick" and "eased-in-progress" paths share one
    /// writer and one definition of which fields emotion owns.
    const fn apply(entity: &mut Entity, s: StyleTarget) {
        entity.face.style.eye_curve = s.eye_curve;
        entity.face.style.mouth_curve = s.mouth_curve;
        entity.face.style.cheek_blush = s.cheek_blush;
        entity.face.style.eye_scale = s.eye_scale;
        entity.face.style.blink_rate_scale = s.blink_rate_scale;
        entity.face.style.breath_depth_scale = s.breath_depth_scale;
        entity.face.left_eye.open_weight = s.open_weight;
        entity.face.right_eye.open_weight = s.open_weight;
        entity.face.mouth.weight = s.mouth_weight;
    }

    /// Produce the interpolated `StyleTarget` between `from` and `to` at
    /// the given fraction of the transition window.
    fn blend(from: StyleTarget, to: StyleTarget, elapsed_ms: u64, duration_ms: u64) -> StyleTarget {
        StyleTarget {
            eye_curve: clamp_i8(ease(
                i32::from(from.eye_curve),
                i32::from(to.eye_curve),
                elapsed_ms,
                duration_ms,
            )),
            mouth_curve: clamp_i8(ease(
                i32::from(from.mouth_curve),
                i32::from(to.mouth_curve),
                elapsed_ms,
                duration_ms,
            )),
            cheek_blush: clamp_to_u8(
                ease(
                    i32::from(from.cheek_blush),
                    i32::from(to.cheek_blush),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                u8::MAX,
            ),
            eye_scale: clamp_to_u8(
                ease(
                    i32::from(from.eye_scale),
                    i32::from(to.eye_scale),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                u8::MAX,
            ),
            blink_rate_scale: clamp_to_u8(
                ease(
                    i32::from(from.blink_rate_scale),
                    i32::from(to.blink_rate_scale),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                u8::MAX,
            ),
            breath_depth_scale: clamp_to_u8(
                ease(
                    i32::from(from.breath_depth_scale),
                    i32::from(to.breath_depth_scale),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                u8::MAX,
            ),
            open_weight: clamp_to_u8(
                ease(
                    i32::from(from.open_weight),
                    i32::from(to.open_weight),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                100,
            ),
            mouth_weight: clamp_to_u8(
                ease(
                    i32::from(from.mouth_weight),
                    i32::from(to.mouth_weight),
                    elapsed_ms,
                    duration_ms,
                ),
                0,
                100,
            ),
        }
    }
}

impl Default for StyleFromEmotion {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for StyleFromEmotion {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "StyleFromEmotion",
            description: "Translates mind.affect.emotion into face.style fields (curves, scales, \
                          cheek blush, open_weight) with linear easing over the transition window.",
            phase: Phase::Expression,
            priority: -10,
            reads: &[Field::Emotion],
            writes: &[
                Field::EyeCurve,
                Field::MouthCurve,
                Field::CheekBlush,
                Field::EyeScale,
                Field::BlinkRateScale,
                Field::BreathDepthScale,
                Field::LeftEyeOpenWeight,
                Field::RightEyeOpenWeight,
                Field::MouthWeight,
            ],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let desired = entity.mind.affect.emotion;
        let desired_target = targets_for(desired);

        // Detect transitions: first tick, or `entity.mind.affect.emotion` changed
        // since the last tick.
        let emotion_changed = self.to_emotion != Some(desired);

        if emotion_changed {
            // Capture the *current* in-flight blended state as the new
            // `from`. On first ever tick there's nothing in flight, so
            // we fall back to the desired target itself, which makes
            // the first-tick snap free.
            let from = match (self.from, self.to, self.transition_start) {
                (Some(f), Some(t), Some(start)) => {
                    let elapsed = now.saturating_duration_since(start);
                    Self::blend(f, t, elapsed, self.transition_ms)
                }
                _ => desired_target,
            };
            self.from = Some(from);
            self.to = Some(desired_target);
            self.to_emotion = Some(desired);
            self.transition_start = Some(now);
        }

        let (Some(from), Some(to), Some(start)) = (self.from, self.to, self.transition_start)
        else {
            // Unreachable in practice -- the branch above always fills
            // all three before we read them. If it ever isn't, apply the
            // target directly rather than leaving the face frozen.
            Self::apply(entity, desired_target);
            return;
        };

        let elapsed = now.saturating_duration_since(start);
        let blended = Self::blend(from, to, elapsed, self.transition_ms);
        Self::apply(entity, blended);
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn first_tick_snaps_to_desired_emotion() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut style = StyleFromEmotion::new();
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);

        let happy = targets_for(Emotion::Happy);
        assert_eq!(entity.face.style.eye_curve, happy.eye_curve);
        assert_eq!(entity.face.style.mouth_curve, happy.mouth_curve);
        assert_eq!(entity.face.style.cheek_blush, happy.cheek_blush);
    }

    #[test]
    fn easing_interpolates_over_transition_window() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Neutral;
        let mut style = StyleFromEmotion::with_transition_ms(300);

        // Establish Neutral as both from and to.
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);

        // Switch to Happy and drive through the transition.
        entity.mind.affect.emotion = Emotion::Happy;
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);

        // Halfway through: eye_curve should be ~35 (half of 70).
        entity.tick.now = Instant::from_millis(150);
        style.update(&mut entity);
        let half = entity.face.style.eye_curve;
        assert!(
            (25..=45).contains(&half),
            "expected eye_curve near 35 at half transition, got {half}"
        );

        // At the end: pinned to the full Happy target.
        entity.tick.now = Instant::from_millis(300);
        style.update(&mut entity);
        let happy = targets_for(Emotion::Happy);
        assert_eq!(entity.face.style.eye_curve, happy.eye_curve);
        assert_eq!(entity.face.style.cheek_blush, happy.cheek_blush);
    }

    #[test]
    fn mid_transition_emotion_change_restarts_cleanly() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Neutral;
        let mut style = StyleFromEmotion::with_transition_ms(300);
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);

        // Start a Happy transition, then interrupt halfway to Sad.
        entity.mind.affect.emotion = Emotion::Happy;
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);
        entity.tick.now = Instant::from_millis(150);
        style.update(&mut entity);
        let mid_curve = entity.face.style.eye_curve;

        entity.mind.affect.emotion = Emotion::Sad;
        entity.tick.now = Instant::from_millis(150);
        style.update(&mut entity);
        // The blended `from` should be the mid-transition snapshot, not
        // the original Neutral -- so the face never jumps.
        assert_eq!(entity.face.style.eye_curve, mid_curve);

        // After the full transition elapses, we're pinned to Sad.
        entity.tick.now = Instant::from_millis(450);
        style.update(&mut entity);
        let sad = targets_for(Emotion::Sad);
        assert_eq!(entity.face.style.eye_curve, sad.eye_curve);
    }

    #[test]
    fn surprised_suppresses_blink_rate() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Surprised;
        let mut style = StyleFromEmotion::new();

        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);
        // After the transition elapses, Surprised's `blink_rate_scale = 0`
        // fully propagates.
        entity.tick.now = Instant::from_millis(StyleFromEmotion::TRANSITION_MS);
        style.update(&mut entity);
        assert_eq!(entity.face.style.blink_rate_scale, 0);
        assert_eq!(entity.face.mouth.weight, 100);
    }

    #[test]
    fn sleepy_droops_eye_open_weight() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Sleepy;
        let mut style = StyleFromEmotion::new();
        entity.tick.now = Instant::from_millis(0);
        style.update(&mut entity);
        entity.tick.now = Instant::from_millis(StyleFromEmotion::TRANSITION_MS);
        style.update(&mut entity);
        let sleepy = targets_for(Emotion::Sleepy);
        assert_eq!(entity.face.left_eye.open_weight, sleepy.open_weight);
        assert_eq!(entity.face.right_eye.open_weight, sleepy.open_weight);
    }

    #[test]
    fn ease_clamps_to_target_after_duration() {
        assert_eq!(ease(0, 100, 0, 300), 0);
        assert_eq!(ease(0, 100, 300, 300), 100);
        assert_eq!(ease(0, 100, 450, 300), 100);
        assert_eq!(ease(100, 0, 150, 300), 50);
    }

    /// Snap through each `Emotion` variant and assert the writer
    /// produces a distinguishable style snapshot. Catches a copy-paste
    /// where two variants accidentally end up with identical
    /// `StyleTarget` rows.
    #[test]
    fn every_emotion_has_a_distinct_style_target() {
        // Compare every pair. Any two variants that produce identical
        // `StyleTarget` rows are a bug — the catalogue would have a
        // visually invisible variant.
        for (i, &a) in Emotion::ALL.iter().enumerate() {
            for &b in &Emotion::ALL[i + 1..] {
                assert_ne!(
                    targets_for(a),
                    targets_for(b),
                    "{a:?} and {b:?} share a StyleTarget row — palettes must differ"
                );
            }
        }
    }

    /// `Hi` is a louder `Happy`: bigger eye scale + faster blink. Pin
    /// the relationship so a future palette tweak can't quietly swap
    /// them along the engagement axis.
    #[test]
    fn hi_is_more_excited_than_happy() {
        let h = targets_for(Emotion::Hi);
        let happy = targets_for(Emotion::Happy);
        assert!(
            h.eye_scale > happy.eye_scale,
            "Hi should widen eyes more than Happy ({} vs {})",
            h.eye_scale,
            happy.eye_scale,
        );
        assert!(
            h.blink_rate_scale > happy.blink_rate_scale,
            "Hi should blink faster than Happy ({} vs {})",
            h.blink_rate_scale,
            happy.blink_rate_scale,
        );
    }

    /// `Loved` is the heaviest-blush variant — the decorator layer
    /// stacks heart overlays on top, but the base style needs to stand
    /// alone too.
    #[test]
    fn loved_has_the_heaviest_blush() {
        let loved = targets_for(Emotion::Loved);
        for &other in Emotion::ALL {
            if other == Emotion::Loved {
                continue;
            }
            assert!(
                loved.cheek_blush >= targets_for(other).cheek_blush,
                "Loved should have the heaviest cheek_blush; {other:?} has {}",
                targets_for(other).cheek_blush,
            );
        }
    }

    /// `Mad` and `Angry` differ along the intensity axis — `Mad` is
    /// the hotter one. Pin the inequality so a palette tweak can't
    /// quietly invert the relationship.
    #[test]
    fn mad_is_hotter_than_angry() {
        let mad = targets_for(Emotion::Mad);
        let angry = targets_for(Emotion::Angry);
        assert!(
            mad.mouth_curve < angry.mouth_curve,
            "Mad should frown deeper than Angry ({} vs {})",
            mad.mouth_curve,
            angry.mouth_curve,
        );
        assert!(
            mad.cheek_blush > angry.cheek_blush,
            "Mad should blush more than Angry ({} vs {})",
            mad.cheek_blush,
            angry.cheek_blush,
        );
        assert!(
            mad.breath_depth_scale >= angry.breath_depth_scale,
            "Mad should breathe at least as deeply as Angry ({} vs {})",
            mad.breath_depth_scale,
            angry.breath_depth_scale,
        );
    }

    /// `Doubt` and `Confused` mirror each other along the valence
    /// axis: same uncertainty signature, opposite mouth sign.
    #[test]
    fn doubt_and_confused_share_uncertainty_oppose_valence() {
        let doubt = targets_for(Emotion::Doubt);
        let confused = targets_for(Emotion::Confused);
        // Both are below baseline on eye_curve (uncertain).
        assert!(
            doubt.eye_curve < 0 && confused.eye_curve < 0,
            "both should drop eye_curve below 0; got Doubt {} / Confused {}",
            doubt.eye_curve,
            confused.eye_curve,
        );
        // Mouth sign disambiguates the pair.
        assert!(
            doubt.mouth_curve > 0 && confused.mouth_curve < 0,
            "Doubt should smile faintly, Confused should frown; got {} / {}",
            doubt.mouth_curve,
            confused.mouth_curve,
        );
    }

    /// `Boring` and `Sleepy` share the half-lidded look but differ on
    /// `mouth_curve` and droop depth. Pin the distinguishing markers so
    /// a future palette tweak doesn't collapse them into one.
    #[test]
    fn boring_is_distinguishable_from_sleepy() {
        let boring = targets_for(Emotion::Boring);
        let sleepy = targets_for(Emotion::Sleepy);
        assert_ne!(
            boring.mouth_curve, sleepy.mouth_curve,
            "Boring's faint frown should distinguish it from Sleepy's flat mouth"
        );
        assert!(
            boring.open_weight > sleepy.open_weight,
            "Sleepy should droop more than Boring ({} vs {})",
            sleepy.open_weight,
            boring.open_weight,
        );
    }

    /// `Curious` widens eyes more than baseline but less than `Surprised`.
    /// Surprised is the wide-eye startle; Curious is the narrower
    /// investigative gaze.
    #[test]
    fn curious_widens_less_than_surprised() {
        let curious = targets_for(Emotion::Curious);
        let surprised = targets_for(Emotion::Surprised);
        assert!(
            curious.eye_scale > SCALE_DEFAULT,
            "Curious should be wider than baseline ({} vs {SCALE_DEFAULT})",
            curious.eye_scale,
        );
        assert!(
            curious.eye_scale < surprised.eye_scale,
            "Curious should be narrower than Surprised ({} vs {})",
            curious.eye_scale,
            surprised.eye_scale,
        );
        // Surprised holds an open-mouth ellipse; Curious doesn't.
        assert_eq!(curious.mouth_weight, 0);
        assert!(surprised.mouth_weight > 0);
    }
}
