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
//! [`Avatar::manual_until`] is already set by another modifier, the
//! `RemoteCommand` stands down. Otherwise it sets the emotion from
//! the table + writes `manual_until = now + MANUAL_HOLD_MS`.
//!
//! [`Avatar::manual_until`]: crate::avatar::Avatar::manual_until

use super::{MANUAL_HOLD_MS, Modifier};
use crate::avatar::Avatar;
use crate::clock::Instant;
use crate::emotion::Emotion;

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

/// Modifier that watches for IR-remote commands queued from the
/// firmware's RMT-RX task and sets emotion per a lookup table.
///
/// Like [`super::EmotionTouch`], the modifier is edge-triggered: the
/// queued command is cleared by the next `update()` call, so a
/// single `queue()` produces exactly one emotion change.
#[derive(Debug, Clone, Copy)]
pub struct RemoteCommand {
    /// Per-remote mapping table. Empty by default, which means the
    /// modifier is a no-op until populated.
    mapping: &'static [RemoteMapping],
    /// Most-recently queued `(address, command)` pair, or `None`.
    pending: Option<(u16, u8)>,
}

impl RemoteCommand {
    /// Construct with an empty mapping. The modifier is a no-op in
    /// this state — useful when the remote codes are still being
    /// discovered, or for sim tests that don't care about IR.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mapping: &[],
            pending: None,
        }
    }

    /// Construct with the given mapping table.
    #[must_use]
    pub const fn with_mapping(mapping: &'static [RemoteMapping]) -> Self {
        Self {
            mapping,
            pending: None,
        }
    }

    /// Queue an `(address, command)` pair for processing on the next
    /// `update()`. Idempotent within a render tick: later queues
    /// overwrite earlier ones, so the most recent code wins.
    pub const fn queue(&mut self, address: u16, command: u8) {
        self.pending = Some((address, command));
    }
}

impl Default for RemoteCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for RemoteCommand {
    fn update(&mut self, avatar: &mut Avatar, now: Instant) {
        let Some((address, command)) = self.pending.take() else {
            return;
        };

        // Another modifier (touch, pickup, ambient) already claimed
        // the emotion. Drop the event on the floor — explicit input
        // wins over remote-control input.
        if let Some(until) = avatar.manual_until
            && now < until
        {
            return;
        }

        // Linear scan of the mapping — table is expected to be tiny
        // (one or two dozen entries at most). First match wins.
        for entry in self.mapping {
            if entry.address == address && entry.command == command {
                avatar.emotion = entry.emotion;
                avatar.manual_until = Some(now + MANUAL_HOLD_MS);
                return;
            }
        }
        // Unknown code: ignore silently. The firmware `ir` task logs
        // every decoded command at info level already, so there's no
        // observability loss here.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Happy);
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(1_000 + MANUAL_HOLD_MS)),
        );
    }

    #[test]
    fn unmapped_code_is_silent_noop() {
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        remote.queue(0x1234, 0x56);
        remote.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn empty_mapping_is_always_noop() {
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::new();
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Neutral);
        assert!(avatar.manual_until.is_none());
    }

    #[test]
    fn queued_command_collapses_to_latest() {
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        // Queue Happy, then overwrite with Sad before the next update.
        remote.queue(0xFF00, 0x01);
        remote.queue(0xFF00, 0x02);
        remote.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(avatar.emotion, Emotion::Sad);
    }

    #[test]
    fn update_consumes_queued_command() {
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(0));
        // Simulate the hold expiring + being cleared by EmotionTouch.
        avatar.manual_until = None;
        avatar.emotion = Emotion::Neutral;
        // Another update with no new queued command must be a no-op.
        remote.update(&mut avatar, Instant::from_millis(100_000));
        assert_eq!(avatar.emotion, Emotion::Neutral);
    }

    #[test]
    fn active_hold_blocks_remote() {
        let mut avatar = Avatar {
            emotion: Emotion::Surprised,
            manual_until: Some(Instant::from_millis(30_000)),
            ..Avatar::default()
        };
        let mut remote = RemoteCommand::with_mapping(MAPPING);
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(1_000));
        assert_eq!(
            avatar.emotion,
            Emotion::Surprised,
            "touch / pickup / ambient hold must outrank remote",
        );
        assert_eq!(
            avatar.manual_until,
            Some(Instant::from_millis(30_000)),
            "hold deadline must be preserved",
        );
    }

    #[test]
    fn remote_fires_after_hold_expires() {
        let mut avatar = Avatar::default();
        let mut remote = RemoteCommand::with_mapping(MAPPING);

        // Hold set by (say) touch in the recent past.
        avatar.manual_until = Some(Instant::from_millis(1_000));
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(500));
        // Command was consumed but had no effect because the hold was
        // active. Clear and try again.
        avatar.manual_until = None;
        remote.queue(0xFF00, 0x01);
        remote.update(&mut avatar, Instant::from_millis(2_000));
        assert_eq!(avatar.emotion, Emotion::Happy);
    }
}
