//! One-frame fire flags propagating modifier-detected edges to the
//! firmware's render loop.
//!
//! [`Events`] is reserved for one-frame edge signals that don't fit the
//! richer [`crate::voice::Voice`] surface. Today it's empty — chirp
//! edges (pickup, wake, low-battery alert) all flow through
//! [`crate::voice::Voice::chirp_request`], which carries the chirp
//! *kind* alongside the edge.
//!
//! ## Lifecycle
//!
//! [`crate::Director::run`] **clears** `entity.events` at the start of
//! each frame, before any modifier runs. Modifiers set fields to `true`
//! when their state machine fires an edge. The firmware reads them
//! after the modifier pass; on the next frame, the Director clears them
//! again.
//!
//! No modifier should ever read `entity.events.*` from the *current*
//! frame — they're firmware-facing signals, not inter-modifier
//! communication. (Inter-modifier coordination flows through
//! [`crate::mind::Affect`] / [`crate::mind::Autonomy`] /
//! [`crate::voice::Voice`].)

/// One-frame fire flags. Cleared by [`crate::Director::run`] at frame
/// start; set by modifiers; read by firmware between modifier pass
/// and post-render work.
///
/// Empty today — every modifier-emitted edge currently maps to a
/// [`crate::voice::ChirpKind`] on `entity.voice.chirp_request`. New
/// fields land here when a future signal *isn't* an audio request:
/// for example, "skill X just completed" notifications, or a debug
/// "modifier Y short-circuited" introspection bit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Events {}
