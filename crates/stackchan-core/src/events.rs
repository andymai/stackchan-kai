//! One-frame fire flags from modifiers to the firmware's render loop.
//!
//! [`Events`] is currently empty: chirp edges flow through
//! [`crate::voice::Voice::chirp_request`] instead, which carries the
//! chirp kind alongside the edge. This struct stays as the slot for
//! signals that aren't audio requests.
//!
//! [`crate::Director::run`] clears `entity.events` at the start of each
//! frame, before any modifier runs. Modifiers set fields when their
//! state machine fires an edge; the firmware reads them after the
//! modifier pass; the next frame clears again.
//!
//! Modifiers don't read `entity.events.*` from the current frame —
//! these are firmware-facing signals, not inter-modifier coordination.
//! Inter-modifier coordination flows through [`crate::mind::Affect`] /
//! [`crate::mind::Autonomy`] / [`crate::voice::Voice`].

/// One-frame fire flags. Cleared by [`crate::Director::run`] at frame
/// start; set by modifiers; read by firmware between the modifier
/// pass and post-render work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Events {}
