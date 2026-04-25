//! `RemoteCommand`: consumes IR-remote events and maps them to
//! emotions via a user-supplied lookup table.
//!
//! ## Why a mapping table instead of fixed behavior?
//!
//! IR remotes are per-user hardware: the NEC `(address, command)`
//! pair your Apple TV remote uses is different from a cheap eBay
//! ESP-family remote, which is different again from an LG / Sony
//! TV. The right UX — and the right codes — can only be decided
//! once you know which remote is on the user's desk. So this
//! modifier takes the mapping as data: a `&'static [(address,
//! command, emotion)]` slice passed in at construction.
//!
//! The `examples/ir_bench.rs` firmware binary prints every decoded
//! NEC frame over defmt so users can populate the table for their
//! specific remote in one sitting.
//!
//! ## Coordination
//!
//! Follows the same "explicit input wins" convention as
//! [`super::EmotionTouch`] and [`super::PickupReaction`]: if
//! `entity.mind.autonomy.manual_until` is already set by another
//! modifier, the `RemoteCommand` stands down. Otherwise it sets the
//! emotion from the table + writes `manual_until = now + MANUAL_HOLD_MS`.
//!
//! Pending input lives on `entity.input.remote_pending`. The firmware's
//! IR task drains the RMT-RX signal and writes
//! `entity.input.remote_pending = Some((address, command))`. This
//! modifier reads + clears the field on each tick.

use super::MANUAL_HOLD_MS;
use crate::director::{Field, ModifierMeta, Phase};
use crate::emotion::Emotion;
use crate::entity::Entity;
use crate::modifier::Modifier;

/// One entry in the remote-command-to-emotion lookup table.
///
/// `address` and `command` are the IR-remote's NEC-protocol codes as
/// decoded by `ir-nec`. Callers that want to match multiple codes to
/// the same emotion just add multiple entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteMapping {
    /// NEC address field.
    pub address: u16,
    /// NEC command field.
    pub command: u8,
    /// Emotion to set when this code is received.
    pub emotion: Emotion,
}

/// Modifier that watches `entity.input.remote_pending` and sets emotion
/// per a lookup table.
///
/// Stateless apart from the immutable mapping: input lives in
/// `entity.input.remote_pending`; the modifier reads + clears it on
/// each tick.
#[derive(Debug, Clone, Copy)]
pub struct RemoteCommand {
    /// Per-remote mapping table. Empty by default, which means the
    /// modifier is a no-op until populated.
    mapping: &'static [RemoteMapping],
}

impl RemoteCommand {
    /// Construct with an empty mapping. The modifier is a no-op in
    /// this state — useful when the remote codes are still being
    /// discovered, or for sim tests that don't care about IR.
    #[must_use]
    pub const fn new() -> Self {
        Self { mapping: &[] }
    }

    /// Construct with the given mapping table.
    #[must_use]
    pub const fn with_mapping(mapping: &'static [RemoteMapping]) -> Self {
        Self { mapping }
    }
}

impl Default for RemoteCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for RemoteCommand {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "RemoteCommand",
            description: "Maps entity.input.remote_pending (address, command) pairs to emotions \
                          via a user-supplied table. Stands down when an earlier modifier already \
                          set mind.autonomy.manual_until.",
            phase: Phase::Affect,
            priority: -90,
            reads: &[Field::Autonomy, Field::RemotePending],
            writes: &[Field::Emotion, Field::Autonomy, Field::RemotePending],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        let now = entity.tick.now;
        let Some((address, command)) = entity.input.remote_pending.take() else {
            return;
        };

        if let Some(until) = entity.mind.autonomy.manual_until
            && now < until
        {
            return;
        }

        for entry in self.mapping {
            if entry.address == address && entry.command == command {
                entity.mind.affect.emotion = entry.emotion;
                entity.mind.autonomy.manual_until = Some(now + MANUAL_HOLD_MS);
                entity.mind.autonomy.source = Some(crate::mind::OverrideSource::Remote);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Instant;

    const MAPPING: &[RemoteMapping] = &[
        RemoteMapping {
            address: 0xFF00,
            command: 0x01,
            emotion: Emotion::Happy,
        },
        RemoteMapping {
            address: 0xFF00,
            command: 0x02,
            emotion: Emotion::Sad,
        },
    ];

    #[test]
    fn mapped_code_sets_emotion_and_hold() {
        let mut entity = Entity::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(1_000);
        remote.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(1_000 + MANUAL_HOLD_MS)),
        );
        assert!(
            entity.input.remote_pending.is_none(),
            "modifier must clear input.remote_pending after consuming"
        );
    }

    #[test]
    fn unmapped_code_is_silent_noop() {
        let mut entity = Entity::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        entity.input.remote_pending = Some((0x1234, 0x56));
        entity.tick.now = Instant::from_millis(1_000);
        remote.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn empty_mapping_is_always_noop() {
        let mut entity = Entity::default();
        let mut remote = RemoteCommand::new();
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(1_000);
        remote.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
        assert!(entity.mind.autonomy.manual_until.is_none());
    }

    #[test]
    fn update_consumes_queued_command() {
        let mut entity = Entity::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(0);
        remote.update(&mut entity);
        // Simulate the hold expiring + being cleared by EmotionTouch.
        entity.mind.autonomy.manual_until = None;
        entity.mind.affect.emotion = Emotion::Neutral;
        // Another update with no new queued command must be a no-op.
        entity.tick.now = Instant::from_millis(100_000);
        remote.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Neutral);
    }

    #[test]
    fn active_hold_blocks_remote() {
        let mut entity = {
            let mut e = Entity::default();
            e.mind.affect.emotion = Emotion::Surprised;
            e.mind.autonomy.manual_until = Some(Instant::from_millis(30_000));
            e
        };
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(1_000);
        remote.update(&mut entity);
        assert_eq!(
            entity.mind.affect.emotion,
            Emotion::Surprised,
            "touch / pickup / ambient hold must outrank remote",
        );
        assert_eq!(
            entity.mind.autonomy.manual_until,
            Some(Instant::from_millis(30_000)),
            "hold deadline must be preserved",
        );
    }

    #[test]
    fn remote_fires_after_hold_expires() {
        let mut entity = Entity::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);

        // Hold set by (say) touch in the recent past.
        entity.mind.autonomy.manual_until = Some(Instant::from_millis(1_000));
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(500);
        remote.update(&mut entity);
        // Command was consumed but had no effect because the hold was
        // active. Clear and try again.
        entity.mind.autonomy.manual_until = None;
        entity.input.remote_pending = Some((0xFF00, 0x01));
        entity.tick.now = Instant::from_millis(2_000);
        remote.update(&mut entity);
        assert_eq!(entity.mind.affect.emotion, Emotion::Happy);
    }
}
