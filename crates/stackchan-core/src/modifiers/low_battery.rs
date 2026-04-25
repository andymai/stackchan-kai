//! `LowBatteryEmotion`: forces `Emotion::Sleepy` when the AXP2101's
//! reported battery state-of-charge drops below a threshold.
//!
//! ## Detection shape
//!
//! Reads `avatar.battery_percent` each tick. Two-threshold hysteresis
//! mirrors [`super::AmbientSleepy`]:
//!
//! - **Enter low:** percent below [`LOW_BATTERY_ENTER_PERCENT`] while
//!   not already low.
//! - **Exit low:** percent above [`LOW_BATTERY_EXIT_PERCENT`] while
//!   currently low. Clears this modifier's own state so autonomy
//!   resumes immediately on the next tick.
//! - Between the two thresholds, the modifier holds its current
//!   state — preventing flicker if the chip's `SoC` estimate is
//!   noisy near the trigger.
//!
//! Unknown battery (`battery_percent = None`, i.e. the power task
//! hasn't published a reading yet) is treated as "no information"
//! and never transitions either way.
//!
//! ## USB-power suppression
//!
//! Even when the percent is below the enter threshold, the modifier
//! stands down if `avatar.usb_power_present == Some(true)`. The
//! reasoning: the unit is charging (or running off USB), so going
//! "sleepy" is the wrong UX — it should look attentive while plugged
//! in even with a depleted battery. Unknown USB state
//! (`usb_power_present = None`) is treated as not-charging, so a
//! pre-first-read tick still allows the override.
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
//! and the battery is still low (and not on USB), the next tick
//! re-fires and sets a fresh hold. So Sleepy effectively rolls forward
//! in [`LOW_BATTERY_HOLD_MS`]-sized chunks, mirroring `AmbientSleepy`.
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
pub const LOW_BATTERY_ENTER_PERCENT: u8 = 15;

/// Battery percent above which the modifier exits low-battery state.
///
/// 20% is the hysteresis upper bound: charging back to ≥ 20% releases
/// the override. The 5-percentage-point gap is wide enough to absorb
/// the AXP2101's typical reporting noise (1–2% LSB jitter under the
/// CoreS3's discharge curve) without flicker.
pub const LOW_BATTERY_EXIT_PERCENT: u8 = 20;

/// Backwards-compat alias for the old single-threshold const.
///
/// Kept so downstream code that imported
/// [`LOW_BATTERY_THRESHOLD_PERCENT`] (e.g. firmware's threshold-
/// crossing detector for the alert beep) keeps compiling; new code
/// should use the explicit `ENTER` / `EXIT` consts.
#[deprecated(
    since = "0.6.0",
    note = "use LOW_BATTERY_ENTER_PERCENT or LOW_BATTERY_EXIT_PERCENT"
)]
pub const LOW_BATTERY_THRESHOLD_PERCENT: u8 = LOW_BATTERY_ENTER_PERCENT;

/// How long the low-battery hold pins Sleepy once set, in ms.
///
/// Short (5 s) by design: the modifier re-sets the hold on every
/// low-battery tick, so the effective behaviour is "Sleepy while
/// battery low, resume within 5 s of charging back above the
/// threshold." Mirrors [`super::AMBIENT_HOLD_MS`].
pub const LOW_BATTERY_HOLD_MS: u64 = 5_000;

/// Modifier that watches [`Avatar::battery_percent`] and forces
/// `Emotion::Sleepy` below a threshold, with hysteresis and USB-power
/// suppression.
///
/// Thresholds are configurable per-instance so apps can dial them up
/// or down without recompiling the core crate.
#[derive(Debug, Clone, Copy)]
pub struct LowBatteryEmotion {
    /// Lower threshold (`<`): transitions out of healthy into low.
    pub enter_threshold_percent: u8,
    /// Upper threshold (`>`): transitions out of low back into healthy.
    pub exit_threshold_percent: u8,
    /// Hysteresis state: `true` once we've crossed below the enter
    /// threshold, `false` once we've crossed back above the exit
    /// threshold. Drives the actual override decision each tick.
    is_low: bool,
}

impl LowBatteryEmotion {
    /// Construct with the default thresholds
    /// ([`LOW_BATTERY_ENTER_PERCENT`] / [`LOW_BATTERY_EXIT_PERCENT`]).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            enter_threshold_percent: LOW_BATTERY_ENTER_PERCENT,
            exit_threshold_percent: LOW_BATTERY_EXIT_PERCENT,
            is_low: false,
        }
    }

    /// Construct with custom thresholds. `exit_threshold_percent`
    /// should normally be greater than `enter_threshold_percent` for
    /// real hysteresis; the modifier doesn't enforce this so unusual
    /// configurations (single-threshold by setting them equal) work
    /// without ceremony.
    #[must_use]
    pub const fn with_thresholds(enter_threshold_percent: u8, exit_threshold_percent: u8) -> Self {
        Self {
            enter_threshold_percent,
            exit_threshold_percent,
            is_low: false,
        }
    }

    /// Exposed for tests: whether the modifier currently believes the
    /// battery is in the low state.
    #[cfg(test)]
    const fn is_low(self) -> bool {
        self.is_low
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

        // Hysteresis: update our internal "is_low" belief.
        if !self.is_low && percent < self.enter_threshold_percent {
            self.is_low = true;
        } else if self.is_low && percent > self.exit_threshold_percent {
            self.is_low = false;
        }

        if !self.is_low {
            return;
        }

        // USB power present — the unit is charging or running off USB.
        // Going "sleepy" while plugged in is the wrong UX; suppress.
        // Unknown USB state (None) falls through and lets the override
        // fire, so a still-booting power task doesn't block the cue.
        if avatar.usb_power_present == Some(true) {
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
    fn enter_threshold_boundary_at_exactly_threshold_does_not_fire() {
        // The check is `percent < enter_threshold`, so
        // exactly-at-threshold is considered healthy.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(LOW_BATTERY_ENTER_PERCENT),
            emotion: Emotion::Happy,
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(!modifier.is_low());
    }

    #[test]
    fn enter_threshold_boundary_one_below_fires() {
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(LOW_BATTERY_ENTER_PERCENT - 1),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
        assert!(modifier.is_low());
    }

    #[test]
    fn hysteresis_holds_low_state_within_band() {
        // Once below the enter threshold, the modifier stays low until
        // the percent crosses *above* the exit threshold — even if it
        // climbs above the enter threshold along the way.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(10),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::from_millis(0));
        assert!(modifier.is_low());

        // Climb to mid-band: still low.
        avatar.battery_percent = Some(17);
        modifier.update(&mut avatar, Instant::from_millis(LOW_BATTERY_HOLD_MS + 1));
        assert!(modifier.is_low());

        // Climb just above exit threshold: still low (need *above*, not at).
        avatar.battery_percent = Some(LOW_BATTERY_EXIT_PERCENT);
        modifier.update(
            &mut avatar,
            Instant::from_millis(2 * LOW_BATTERY_HOLD_MS + 1),
        );
        assert!(modifier.is_low());

        // Climb past exit: clears.
        avatar.battery_percent = Some(LOW_BATTERY_EXIT_PERCENT + 1);
        modifier.update(
            &mut avatar,
            Instant::from_millis(3 * LOW_BATTERY_HOLD_MS + 1),
        );
        assert!(!modifier.is_low());
    }

    #[test]
    fn usb_power_suppresses_low_battery_override() {
        // Battery is below threshold but USB is plugged in — modifier
        // tracks "is_low" but does not write Sleepy.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(5),
            usb_power_present: Some(true),
            emotion: Emotion::Happy,
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert!(modifier.is_low());
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn unknown_usb_state_lets_override_fire() {
        // `usb_power_present = None` (pre-first-read) shouldn't block
        // the modifier; we'd rather show low-battery once and correct
        // later than miss the cue.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(5),
            usb_power_present: None,
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
    }

    #[test]
    fn unplugging_usb_releases_suppression() {
        // Plugged in, low battery → no override.
        // Unplug while still low → override fires on the next tick.
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(5),
            usb_power_present: Some(true),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::from_millis(0));
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(modifier.is_low());

        // Unplug.
        avatar.usb_power_present = Some(false);
        modifier.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Sleepy);
    }

    #[test]
    fn custom_thresholds_take_effect() {
        let mut modifier = LowBatteryEmotion::with_thresholds(50, 60);
        let mut avatar = Avatar {
            battery_percent: Some(40),
            ..Avatar::default()
        };
        modifier.update(&mut avatar, Instant::ZERO);
        assert_eq!(avatar.emotion, Emotion::Sleepy);
        assert!(modifier.is_low());

        // Climb to between custom enter/exit: still low.
        avatar.battery_percent = Some(55);
        modifier.update(&mut avatar, Instant::from_millis(LOW_BATTERY_HOLD_MS + 1));
        assert!(modifier.is_low());

        // Climb past custom exit: clears.
        avatar.battery_percent = Some(61);
        modifier.update(
            &mut avatar,
            Instant::from_millis(2 * LOW_BATTERY_HOLD_MS + 1),
        );
        assert!(!modifier.is_low());
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

    #[test]
    fn hysteresis_band_walk_does_not_flicker() {
        // Walking up and down inside the dead-band should not cause
        // emotion thrash. Specifically: enter low, climb into band,
        // dip back below enter, climb again — emotion stays Sleepy
        // throughout (after the initial trigger).
        let mut modifier = LowBatteryEmotion::new();
        let mut avatar = Avatar {
            battery_percent: Some(10),
            ..Avatar::default()
        };

        // Step the time past each hold window so the modifier
        // re-fires; otherwise the manual_until short-circuit
        // would mask any real change of behaviour.
        let mut t = 1_000u64;
        let step = LOW_BATTERY_HOLD_MS + 1;

        for percent in [10, 17, 14, 18, 13, 19, 12] {
            avatar.battery_percent = Some(percent);
            modifier.update(&mut avatar, Instant::from_millis(t));
            assert!(
                modifier.is_low(),
                "percent={percent} flipped is_low off mid-band"
            );
            assert_eq!(
                avatar.emotion,
                Emotion::Sleepy,
                "percent={percent} drove emotion off Sleepy"
            );
            t += step;
        }
    }
}
