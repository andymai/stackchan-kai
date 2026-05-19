//! [`BatteryOverlayFromPerception`] — renders the on-screen battery
//! indicator when the operator has opted in via
//! `BehaviorConfig::battery_icon_enabled`.
//!
//! Quantises [`crate::perception::Perception::battery_percent`] into a
//! [`BatteryBucket`] at the source so frame-to-frame percent jitter
//! doesn't keep flipping the renderer's dirty-check; the bucket
//! changes only when the indicator's visual state actually differs.
//! The charging flag mirrors `perception.usb_power_present`.
//!
//! ## Phase + priority
//!
//! Runs in [`Phase::Decoration`] at priority `5`. The decorator
//! pipeline (expiry at `-10`, triggers at `0`) lives in the same
//! phase, but the battery overlay writes a disjoint field
//! ([`Field::BatteryOverlay`]) so order against the decorator
//! triggers is immaterial.

use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::face::{BatteryBucket, BatteryOverlay};
use crate::modifier::Modifier;

/// Modifier that maps perception's battery percent + USB-power flag
/// into the quantised on-screen indicator. Opt-in via
/// [`Self::with_enabled`].
#[derive(Debug, Clone, Copy)]
pub struct BatteryOverlayFromPerception {
    /// `true` once the operator has opted into the overlay via
    /// `BehaviorConfig::battery_icon_enabled`; mirrored at boot.
    /// `false` short-circuits [`Modifier::update`] to a no-op and
    /// clears any stale overlay.
    enabled: bool,
}

impl BatteryOverlayFromPerception {
    /// Construct with the operator-set enable flag baked in. The
    /// firmware mirrors `BehaviorConfig::battery_icon_enabled` into
    /// the `bool` at boot.
    #[must_use]
    pub const fn with_enabled(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Construct disabled. Equivalent to `with_enabled(false)`.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_enabled(false)
    }
}

impl Default for BatteryOverlayFromPerception {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for BatteryOverlayFromPerception {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "BatteryOverlayFromPerception",
            description: "Maps perception.battery_percent + usb_power_present to \
                          face.battery_overlay; opt-in via \
                          BehaviorConfig::battery_icon_enabled.",
            phase: Phase::Decoration,
            priority: 5,
            reads: &[Field::BatteryPercent, Field::UsbPowerPresent],
            writes: &[Field::BatteryOverlay],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if !self.enabled {
            if entity.face.battery_overlay.is_some() {
                entity.face.battery_overlay = None;
            }
            return;
        }
        let next = entity
            .perception
            .battery_percent
            .map(|percent| BatteryOverlay {
                bucket: BatteryBucket::from_percent(percent),
                charging: entity.perception.usb_power_present.unwrap_or(false),
            });
        if entity.face.battery_overlay != next {
            entity.face.battery_overlay = next;
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: Option::expect on values just written by the \
              modifier-under-test"
)]
mod tests {
    use super::*;

    #[test]
    fn disabled_clears_overlay_and_writes_none_when_already_none() {
        let mut e = Entity::default();
        e.face.battery_overlay = Some(BatteryOverlay {
            bucket: BatteryBucket::Full,
            charging: false,
        });
        let mut m = BatteryOverlayFromPerception::with_enabled(false);
        m.update(&mut e);
        assert!(e.face.battery_overlay.is_none());

        m.update(&mut e);
        assert!(e.face.battery_overlay.is_none());
    }

    #[test]
    fn enabled_without_perception_keeps_overlay_none() {
        let mut e = Entity::default();
        let mut m = BatteryOverlayFromPerception::with_enabled(true);
        m.update(&mut e);
        assert!(e.face.battery_overlay.is_none());
    }

    #[test]
    fn enabled_with_perception_writes_quantised_overlay() {
        let mut e = Entity::default();
        e.perception.battery_percent = Some(42);
        e.perception.usb_power_present = Some(true);
        let mut m = BatteryOverlayFromPerception::with_enabled(true);
        m.update(&mut e);
        let o = e.face.battery_overlay.expect("overlay should be Some");
        assert_eq!(o.bucket, BatteryBucket::Medium);
        assert!(o.charging);
    }

    #[test]
    fn percent_jitter_within_bucket_keeps_overlay_stable() {
        let mut e = Entity::default();
        e.perception.usb_power_present = Some(false);
        let mut m = BatteryOverlayFromPerception::with_enabled(true);
        e.perception.battery_percent = Some(50);
        m.update(&mut e);
        let first = e.face.battery_overlay.expect("set");
        e.perception.battery_percent = Some(73);
        m.update(&mut e);
        let second = e.face.battery_overlay.expect("set");
        assert_eq!(first, second);
    }

    #[test]
    fn charging_flag_follows_usb_power_present() {
        let mut e = Entity::default();
        e.perception.battery_percent = Some(80);
        e.perception.usb_power_present = Some(true);
        let mut m = BatteryOverlayFromPerception::with_enabled(true);
        m.update(&mut e);
        assert!(e.face.battery_overlay.expect("set").charging);

        e.perception.usb_power_present = Some(false);
        m.update(&mut e);
        assert!(!e.face.battery_overlay.expect("set").charging);
    }

    #[test]
    fn new_and_default_construct_disabled_instance() {
        // Both `new()` and `Default::default()` should produce a
        // disabled modifier that no-ops when ticked.
        for ctor_name in ["new", "default"] {
            let mut m = if ctor_name == "new" {
                BatteryOverlayFromPerception::new()
            } else {
                BatteryOverlayFromPerception::default()
            };
            let mut e = Entity::default();
            e.perception.battery_percent = Some(50);
            m.update(&mut e);
            assert!(
                e.face.battery_overlay.is_none(),
                "{ctor_name}: disabled modifier must not write overlay",
            );
        }
    }

    #[test]
    fn modifier_meta_advertises_battery_overlay_writes() {
        // Pin the meta surface so Director's assert_only_writes
        // enforcement keeps matching the actual writes inside update.
        let m = BatteryOverlayFromPerception::new();
        let meta = m.meta();
        assert_eq!(meta.name, "BatteryOverlayFromPerception");
        assert_eq!(meta.phase, Phase::Decoration);
        assert_eq!(meta.priority, 5);
        assert_eq!(meta.reads, &[Field::BatteryPercent, Field::UsbPowerPresent]);
        assert_eq!(meta.writes, &[Field::BatteryOverlay]);
    }
}
