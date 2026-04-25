//! `LowBatteryEmotion`: forces `Emotion::Sleepy` when the AXP2101's
//! reported battery state-of-charge drops below a threshold.
//!
//! ## Detection shape
//!
//! Reads `avatar.battery_percent` each tick. Single-threshold check
//! (no hysteresis); the AXP2101's internal `SoC` estimate is already
//! smoothed and the threshold is conservative enough that bouncing
//! across the boundary should be rare in practice. If hardware
//! observation shows flicker, swap to a two-threshold variant
//! mirroring [`super::AmbientSleepy`].
//!
//! Unknown battery (`battery_percent = None`, i.e. the power task
//! hasn't published a reading yet) is treated as "no information"
//! and never triggers the override.
//!
//! ## Coordination with the other emotion modifiers
//!
//! Like [`super::PickupReaction`] and [`super::AmbientSleepy`], this
//! modifier respects an existing [`Avatar::manual_until`] hold — if
//! touch, a pickup, or any other explicit input has already claimed
//! the emotion, we stand down. Low battery is *background state*: it
//! shouldn't override a user's deliberate interaction.
//!
//! When the modifier fires, it sets a [`LOW_BATTERY_HOLD_MS`] hold.
//! Subsequent ticks short-circuit on the active hold; once it expires
//! and the battery is still low, the next tick re-fires and sets a
//! fresh hold. So Sleepy effectively rolls forward in
//! [`LOW_BATTERY_HOLD_MS`]-sized chunks, mirroring `AmbientSleepy`.
//!
//! [`Avatar::manual_until`]: crate::avatar::Avatar::manual_until

use super::Modifier;
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

/// Battery percent below which `Emotion::Sleepy` is forced.
///
/// 15% is "still has time to find a charger but should stop being cute
/// about it" — chosen so the avatar's behaviour change is a usable
/// hint that the unit needs power, not a panic state.
pub const LOW_BATTERY_THRESHOLD_PERCENT: u8 = 15;

/// How long the low-battery hold pins Sleepy once set, in ms.
///
/// Short (5 s) by design: the modifier re-sets the hold on every
/// low-battery tick, so the effective behaviour is "Sleepy while
/// battery low, resume within 5 s of charging back above the
/// threshold." Mirrors [`super::AMBIENT_HOLD_MS`].
pub const LOW_BATTERY_HOLD_MS: u64 = 5_000;

/// Modifier that watches [`Avatar::battery_percent`] and forces
/// `Emotion::Sleepy` below a threshold.
///
/// Stateless — purely reads the current battery field and writes
/// emotion. The threshold is configurable per-instance so apps can
/// dial the trigger up or down without recompiling the core crate.
#[derive(Debug, Clone, Copy)]
pub struct LowBatteryEmotion {
    /// Trigger threshold; sleepy fires when `battery_percent < threshold`.
    pub threshold_percent: u8,
}

impl LowBatteryEmotion {
    /// Construct with the default threshold ([`LOW_BATTERY_THRESHOLD_PERCENT`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold_percent: LOW_BATTERY_THRESHOLD_PERCENT,
        }
    }

    /// Construct with a custom threshold.
    #[must_use]
    pub const fn with_threshold(threshold_percent: u8) -> Self {
        Self { threshold_percent }
    }
}

impl Default for LowBatteryEmotion {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for LowBatteryEmotion {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let Some(percent) = avatar.battery_percent else {
            // No reading yet — nothing to do.
            return;
        };

        if percent >= self.threshold_percent {
            // Battery healthy; defer to the autonomy stack.
            return;
        }

        // Another modifier (touch, pickup, remote) has already claimed
        // the emotion. Stand down — low battery is background state,
        // explicit input wins.
        if let Some(until) = avatar.manual_until
            && now < until
        {
            return;
        }

        avatar.emotion = Emotion::Sleepy;
        // Set a fresh hold. Subsequent ticks within the hold window
        // short-circuit at the manual_until check above; once the
        // hold expires, the next low-battery tick rolls a new one.
        avatar.manual_until = Some(now + LOW_BATTERY_HOLD_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: avatar with a healthy battery, well above the threshold.
    fn healthy_avatar() -> Avatar {
        Avatar {
            battery_percent: Some(80),
            ..Avatar::default()
        }
    }

    /// Helper: avatar with a low battery, well below the threshold.
    fn low_battery_avatar() -> Avatar {
        Avatar {
            battery_percent: Some(5),
            ..Avatar::default()
        }
    }

    #[test]
    fn no_battery_reading_does_nothing() {
        let mut modifier = LowBatteryEmotion::new();
        // battery_percent = None
        let mut avatar = Avatar {
            emotion: Emotion::Happy,
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn healthy_battery_does_nothing() {
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            emotion: Emotion::Happy,
            ..healthy_avatar()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn low_battery_forces_sleepy() {
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            emotion: Emotion::Happy,
            ..low_battery_avatar()
        };
        let now = Instant::from_millis(1_000);
        modifier.update(&mut avatar, now);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
        assert_eq!(avatar.manual_until, Some(now + LOW_BATTERY_HOLD_MS));
    }

    #[test]
    fn manual_hold_suppresses_low_battery_override() {
        let mut modifier = LowBatteryEmotion::new();
        let hold_deadline = Instant::from_millis(10_000);
        let mut avatar = Avatar {
            // Claimed by e.g. PickupReaction.
            emotion: Emotion::Surprised,
            manual_until: Some(hold_deadline),
            ..low_battery_avatar()
        };
        modifier.update(&mut avatar, Instant::from_millis(5_000));
        // Hold still active — modifier stands down.
        assert_eq!(avatar.emotion, Emotion::Surprised);
        assert_eq!(avatar.manual_until, Some(hold_deadline));
    }

    #[test]
    fn expired_manual_hold_lets_low_battery_through() {
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            manual_until: Some(Instant::from_millis(1_000)),
            ..low_battery_avatar()
        };
        let now = Instant::from_millis(2_000);
        modifier.update(&mut avatar, now);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
        assert_eq!(avatar.manual_until, Some(now + LOW_BATTERY_HOLD_MS));
    }

    #[test]
    fn threshold_boundary_at_exactly_threshold_does_not_fire() {
        // The check is `percent < threshold`, so exactly-at-threshold
        // is considered healthy. Pin this so it doesn't drift.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(LOW_BATTERY_THRESHOLD_PERCENT),
            emotion: Emotion::Happy,
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn threshold_boundary_one_below_fires() {
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(LOW_BATTERY_THRESHOLD_PERCENT - 1),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
    }

    #[test]
    fn custom_threshold_takes_effect() {
        let mut modifier = LowBatteryEmotion::with_threshold(50);
        let mut avatar = Avatar {
            battery_percent: Some(40),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
    }

    #[test]
    fn second_tick_within_hold_window_does_not_refresh_deadline() {
        // Mirrors AmbientSleepy: once the modifier sets a hold, it
        // short-circuits on subsequent ticks until the hold expires.
        // The hold rolls forward in [LOW_BATTERY_HOLD_MS]-sized
        // chunks, not on every tick.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = low_battery_avatar();
        modifier.update(&mut avatar, Instant::from_millis(1_000));
        let first_deadline = avatar.manual_until;
        // Tick again 1 s later — well within the 5 s hold window.
        modifier.update(&mut avatar, Instant::from_millis(2_000));
        assert_eq!(avatar.manual_until, first_deadline);
    }

    #[test]
    fn rolls_hold_forward_after_expiry() {
        // After the first hold expires, a still-low battery should
        // re-fire the modifier and set a fresh hold.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = low_battery_avatar();
        modifier.update(&mut avatar, Instant::from_millis(1_000));
        let first_deadline = avatar.manual_until;
        // Tick after the hold has expired.
        let later = Instant::from_millis(1_000 + LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut avatar, later);
        assert_eq!(avatar.manual_until, Some(later + LOW_BATTERY_HOLD_MS));
        assert!(avatar.manual_until > first_deadline);
    }
}
