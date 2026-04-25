//! Skill implementations.
//!
//! Each skill implements the [`crate::Skill`] trait and registers with
//! a [`crate::Director`] via [`crate::Director::add_skill`]. Skills
//! poll their `should_fire` predicate every frame; matching skills'
//! `invoke` runs in priority order.
//!
//! Skills write `mind.intent` / `mind.attention` / `voice` /
//! `events` — modifiers translate that intent into face / motor.
//! See [`crate::skill`] for the trait contract.
//!
//! ## Catalog
//!
//! - [`LookAtSound`] — sustained `perception.audio_rms` flips
//!   `mind.attention` to `Listening` and `mind.intent` to `Listen`.
//!   Pairs with the [`crate::modifiers::ListenHead`] motion modifier
//!   for a cocked-head listening posture.

mod look_at_sound;
mod petting;

pub use look_at_sound::{
    LISTEN_RELEASE_MS, LISTEN_RMS_THRESHOLD, LISTEN_SUSTAIN_TICKS, LookAtSound,
};
pub use petting::{PETTING_SUSTAIN_TICKS, Petting};
