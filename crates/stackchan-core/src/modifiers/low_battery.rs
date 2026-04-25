//! `LowBatteryEmotion`: forces `Emotion::Sleepy` when the AXP2101's
//! reported battery state-of-charge drops below a threshold, and
//! requests a one-shot alert chirp on the arming edge.
//!
//! ## Detection shape
//!
//! Reads `entity.perception.battery_percent` each tick. Two-threshold
//! hysteresis mirrors [`super::AmbientSleepy`]:
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
//! stands down if `entity.perception.usb_power_present == Some(true)`.
//! The reasoning: the unit is charging (or running off USB), so going
//! "sleepy" is the wrong UX — it should look attentive while plugged
//! in even with a depleted battery. Unknown USB state
//! (`usb_power_present = None`) is treated as not-charging, so a
//! pre-first-read tick still allows the override.
//!
//! ## Alert chirp
//!
//! On the arming edge — the tick on which `is_low` flips from `false`
//! to `true` while unplugged — the modifier sets
//! `entity.voice.chirp_request = Some(ChirpKind::LowBatteryAlert)` so
//! the firmware's audio task plays a short alert beep. Plugging back
//! in and dropping below the threshold again re-arms (after the exit-
//! threshold crossing).
//!
//! ## Coordination with the other emotion modifiers
//!
//! Like [`super::PickupReaction`] and [`super::AmbientSleepy`], this
//! modifier respects an existing `entity.mind.autonomy.manual_until`
//! hold — if touch, a pickup, or any other explicit input has already
//! claimed the emotion, we stand down. Low battery is *background
//! state*: it shouldn't override a user's deliberate interaction.
//!
//! When the modifier fires, it sets a [`LOW_BATTERY_HOLD_MS`] hold.
//! Subsequent ticks short-circuit on the active hold; once it expires
//! and the battery is still low (and not on USB), the next tick
//! re-fires and sets a fresh hold. So Sleepy effectively rolls forward
//! in [`LOW_BATTERY_HOLD_MS`]-sized chunks, mirroring `AmbientSleepy`.

use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;
use crate::voice::ChirpKind;

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

/// How long the low-battery hold pins Sleepy once set, in ms.
///
/// Short (5 s) by design: the modifier re-sets the hold on every
/// low-battery tick, so the effective behaviour is "Sleepy while
/// battery low, resume within 5 s of charging back above the
/// threshold." Mirrors [`super::AMBIENT_HOLD_MS`].
pub const LOW_BATTERY_HOLD_MS: u64 = 5_000;

/// Modifier that watches `entity.perception.battery_percent` and forces
/// `Emotion::Sleepy` below a threshold.
///
/// Has hysteresis and USB-power suppression. On the arming edge it
/// sets `entity.voice.chirp_request = LowBatteryAlert` so the firmware
/// can play a one-shot alert beep. Thresholds are configurable
/// per-instance so apps can dial them up or down without recompiling
/// the core crate.
#[derive(Debug, Clone, Copy)]
pub struct LowBatteryEmotion {
    /// Lower threshold (`<`): transitions out of healthy into low.
    pub enter_threshold_percent: u8,
    /// Upper threshold (`>`): transitions out of low back into healthy.
    pub exit_threshold_percent: u8,
    /// Hysteresis state for the emotion override: `true` once we've
    /// crossed below the enter threshold, `false` once we've crossed
    /// back above the exit threshold.
    is_low: bool,
    /// Edge-detect state for the alert chirp: `true` while the unit
    /// has gone below the enter threshold *while unplugged* and hasn't
    /// since climbed above the exit threshold. Fires the chirp once on
    /// the rising edge; re-arms only after a healthy-charge crossing.
    alert_armed: bool,
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
            alert_armed: false,
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
            alert_armed: false,
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
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "LowBatteryEmotion",
            description: "Hysteresis on perception.battery_percent + usb_power_present: forces \
                          emotion=Sleepy below threshold while unplugged, and sets \
                          voice.chirp_request = LowBatteryAlert on the arming edge.",
            phase: Phase::Affect,
            priority: -50,
            reads: &[
                Field::BatteryPercent,
                Field::UsbPowerPresent,
                Field::Autonomy,
            ],
            writes: &[Field::Emotion, Field::Autonomy, Field::ChirpRequest],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let Some(percent) = entity.perception.battery_percent else {
            // No reading yet — nothing to do.
            return;
        };
        let unplugged = entity.perception.usb_power_present != Some(true);

        // Emotion-override hysteresis.
        if !self.is_low && percent < self.enter_threshold_percent {
            self.is_low = true;
        } else if self.is_low && percent > self.exit_threshold_percent {
            self.is_low = false;
        }

        // Alert-chirp hysteresis. Mirrors the original firmware-side
        // logic: arms the *first* time we see (low percent && unplugged)
        // since boot or since the last healthy-charge crossing. Climbing
        // past the exit threshold rearms the next descent. This means
        // unplugging while already-low fires the chirp (transition into
        // unsafe state) but re-plugging then re-unplugging at the same
        // SoC does not.
        if self.alert_armed {
            if percent > self.exit_threshold_percent {
                self.alert_armed = false;
            }
        } else if percent < self.enter_threshold_percent && unplugged {
            self.alert_armed = true;
            entity.voice.chirp_request = Some(ChirpKind::LowBatteryAlert);
        }

        if !self.is_low || !unplugged {
            // Either healthy, or charging — emotion override stands down.
            return;
        }

        // Another modifier (touch, pickup, remote) has already claimed
        // the emotion. Stand down — low battery is background state,
        // explicit input wins.
        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            return;
        }

        entity.mind.affect.emotion = Emotion::Sleepy;
        entity.mind.autonomy.source = Some(crate::mind::OverrideSource::LowBattery);
        // Set a fresh hold. Subsequent ticks within the hold window
        // short-circuit at the manual_until check above; once the
        // hold expires, the next low-battery tick rolls a new one.
        entity.mind.autonomy.manual_until = Some(now + LOW_BATTERY_HOLD_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    /// Helper: avatar with a healthy battery, well above the threshold.
    fn healthy_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.battery_percent = Some(80);
        e
    }

    /// Helper: avatar with a low battery, well below the threshold.
    fn low_battery_avatar() -> Entity {
        let mut e = Entity::default();
        e.perception.battery_percent = Some(5);
        e
    }

    #[test]
    fn no_battery_reading_does_nothing() {
        let mut modifier = LowBatteryEmotion::new();
        // battery_percent = None
        let mut entity = {
            let mut e = Entity::default();
            e.mind.affect.emotion = Emotion::Happy;
            e
        };
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn healthy_battery_does_nothing() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = {
            let mut e = healthy_avatar();
            e.mind.affect.emotion = Emotion::Happy;
            e
        };
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn low_battery_forces_sleepy() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = {
            let mut e = low_battery_avatar();
            e.mind.affect.emotion = Emotion::Happy;
            e
        };
        let now = Instant::from_millis(1_000);
        entity.tick.now = now;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(now + LOW_BATTERY_HOLD_MS)
        );
    }

    #[test]
    fn manual_hold_suppresses_low_battery_override() {
        let mut modifier = LowBatteryEmotion::new();
        let hold_deadline = Instant::from_millis(10_000);
        let mut entity = {
            let mut e = low_battery_avatar();
            e.mind.affect.emotion = Emotion::Surprised;
            e.mind.autonomy.manual_until = Some(hold_deadline);
            e
        };
        entity.tick.now = Instant::from_millis(5_000);
        modifier.update(&mut entity);
        // Hold still active — modifier stands down.
        assert_eq!(entity.mind.affect.emotion, Emotion::Surprised);
        assert_eq!(entity.mind.autonomy.manual_until, Some(hold_deadline));
    }

    #[test]
    fn expired_manual_hold_lets_low_battery_through() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(1_000));
        let now = Instant::from_millis(2_000);
        entity.tick.now = now;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(now + LOW_BATTERY_HOLD_MS)
        );
    }

    #[test]
    fn enter_threshold_boundary_at_exactly_threshold_does_not_fire() {
        // The check is `percent < enter_threshold`, so
        // exactly-at-threshold is considered healthy.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = Entity::default();
        entity.perception.battery_percent = Some(LOW_BATTERY_ENTER_PERCENT);
        entity.mind.affect.emotion = Emotion::Happy;
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(!modifier.is_low());
    }

    #[test]
    fn enter_threshold_boundary_one_below_fires() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = {
            let mut e = Entity::default();
            e.perception.battery_percent = Some(LOW_BATTERY_ENTER_PERCENT - 1);
            e
        };
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
        assert!(modifier.is_low());
    }

    #[test]
    fn hysteresis_holds_low_state_within_band() {
        // Once below the enter threshold, the modifier stays low until
        // the percent crosses *above* the exit threshold — even if it
        // climbs above the enter threshold along the way.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = {
            let mut e = Entity::default();
            e.perception.battery_percent = Some(10);
            e
        };
        entity.tick.now = Instant::from_millis(0);
        modifier.update(&mut entity);
        assert!(modifier.is_low());

        // Climb to mid-band: still low.
        entity.perception.battery_percent = Some(17);
        entity.tick.now = Instant::from_millis(LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(modifier.is_low());

        // Climb just above exit threshold: still low (need *above*, not at).
        entity.perception.battery_percent = Some(LOW_BATTERY_EXIT_PERCENT);
        entity.tick.now = Instant::from_millis(2 * LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(modifier.is_low());

        // Climb past exit: clears.
        entity.perception.battery_percent = Some(LOW_BATTERY_EXIT_PERCENT + 1);
        entity.tick.now = Instant::from_millis(3 * LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(!modifier.is_low());
    }

    #[test]
    fn usb_power_suppresses_low_battery_override() {
        // Battery is below threshold but USB is plugged in — modifier
        // tracks "is_low" but does not write Sleepy.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = Entity::default();
        entity.perception.battery_percent = Some(5);
        entity.perception.usb_power_present = Some(true);
        entity.mind.affect.emotion = Emotion::Happy;
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert!(modifier.is_low());
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn unknown_usb_state_lets_override_fire() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = Entity::default();
        entity.perception.battery_percent = Some(5);
        entity.perception.usb_power_present = None;
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
    }

    #[test]
    fn unplugging_usb_releases_suppression() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = Entity::default();
        entity.perception.battery_percent = Some(5);
        entity.perception.usb_power_present = Some(true);
        entity.tick.now = Instant::from_millis(0);
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(modifier.is_low());

        // Unplug.
        entity.perception.usb_power_present = Some(false);
        entity.tick.now = Instant::from_millis(1_000);
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
    }

    #[test]
    fn custom_thresholds_take_effect() {
        let mut modifier = LowBatteryEmotion::with_thresholds(50, 60);
        let mut entity = {
            let mut e = Entity::default();
            e.perception.battery_percent = Some(40);
            e
        };
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Sleepy);
        assert!(modifier.is_low());

        // Climb to between custom enter/exit: still low.
        entity.perception.battery_percent = Some(55);
        entity.tick.now = Instant::from_millis(LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(modifier.is_low());

        // Climb past custom exit: clears.
        entity.perception.battery_percent = Some(61);
        entity.tick.now = Instant::from_millis(2 * LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(!modifier.is_low());
    }

    #[test]
    fn second_tick_within_hold_window_does_not_refresh_deadline() {
        // Mirrors AmbientSleepy: once the modifier sets a hold, it
        // short-circuits on subsequent ticks until the hold expires.
        // The hold rolls forward in [LOW_BATTERY_HOLD_MS]-sized
        // chunks, not on every tick.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.tick.now = Instant::from_millis(1_000);
        modifier.update(&mut entity);
        let first_deadline = entity.mind.autonomy.manual_until;
        // Tick again 1 s later — well within the 5 s hold window.
        entity.tick.now = Instant::from_millis(2_000);
        modifier.update(&mut entity);
        assert_eq!(entity.mind.autonomy.manual_until, first_deadline);
    }

    #[test]
    fn rolls_hold_forward_after_expiry() {
        // After the first hold expires, a still-low battery should
        // re-fire the modifier and set a fresh hold.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.tick.now = Instant::from_millis(1_000);
        modifier.update(&mut entity);
        let first_deadline = entity.mind.autonomy.manual_until;
        // Tick after the hold has expired.
        let later = Instant::from_millis(1_000 + LOW_BATTERY_HOLD_MS + 1);
        entity.tick.now = later;
        modifier.update(&mut entity);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(later + LOW_BATTERY_HOLD_MS)
        );
        assert!(entity.mind.autonomy.manual_until > first_deadline);
    }

    #[test]
    fn arming_edge_fires_alert_chirp_once() {
        // Boot already low + unplugged. First tick fires the alert; a
        // continued low-percent tick does NOT re-fire (still armed).
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.perception.usb_power_present = Some(false);
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::LowBatteryAlert));

        // Simulate firmware draining the request, then tick again —
        // still low + unplugged but already armed, so no fresh chirp.
        entity.voice.chirp_request = None;
        entity.tick.now = Instant::from_millis(1_000);
        modifier.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());
    }

    #[test]
    fn alert_rearms_after_healthy_crossing() {
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.perception.usb_power_present = Some(false);
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::LowBatteryAlert));
        entity.voice.chirp_request = None;

        // Charge back above the exit threshold — clears alert_armed.
        entity.perception.battery_percent = Some(LOW_BATTERY_EXIT_PERCENT + 5);
        entity.perception.usb_power_present = Some(true);
        entity.tick.now = Instant::from_millis(LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());

        // Drop and unplug again — alert fires.
        entity.perception.battery_percent = Some(5);
        entity.perception.usb_power_present = Some(false);
        entity.tick.now = Instant::from_millis(2 * LOW_BATTERY_HOLD_MS + 1);
        modifier.update(&mut entity);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::LowBatteryAlert));
    }

    #[test]
    fn plugged_in_does_not_fire_alert() {
        // Boot below threshold but plugged in — alert must not fire,
        // emotion override must not engage. Subsequent unplug while
        // still low fires the alert.
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = low_battery_avatar();
        entity.perception.usb_power_present = Some(true);
        entity.mind.affect.emotion = Emotion::Happy;
        entity.tick.now = Instant::ZERO;
        modifier.update(&mut entity);
        assert!(entity.voice.chirp_request.is_none());
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);

        // Unplug → alert fires.
        entity.perception.usb_power_present = Some(false);
        entity.tick.now = Instant::from_millis(1_000);
        modifier.update(&mut entity);
        assert_eq!(entity.voice.chirp_request, Some(ChirpKind::LowBatteryAlert));
    }

    #[test]
    fn hysteresis_band_walk_does_not_flicker() {
        // Walking up and down inside the dead-band should not cause
        // emotion thrash. Specifically: enter low, climb into band,
        // dip back below enter, climb again — emotion stays Sleepy
        // throughout (after the initial trigger).
        let mut modifier = LowBatteryEmotion::new();
        let mut entity = {
            let mut e = Entity::default();
            e.perception.battery_percent = Some(10);
            e
        };

        // Step the time past each hold window so the modifier
        // re-fires; otherwise the manual_until short-circuit
        // would mask any real change of behaviour.
        let mut t = 1_000u64;
        let step = LOW_BATTERY_HOLD_MS + 1;

        for percent in [10, 17, 14, 18, 13, 19, 12] {
            entity.perception.battery_percent = Some(percent);
            entity.tick.now = Instant::from_millis(t);
            modifier.update(&mut entity);
            assert!(
                modifier.is_low(),
                "percent={percent} flipped is_low off mid-band"
            );
            assert_eq!(
                entity.mind.affect.emotion,
                Emotion::Sleepy,
                "percent={percent} drove emotion off Sleepy"
            );
            t += step;
        }
    }
}
