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
//! See each skill's module docs for the full description; this list is
//! a quick map of which input drives which intent. Add new skills by
//! dropping a module here and registering it on the [`crate::Director`].

mod handling;
mod look_at_sound;
mod petting;

pub use handling::{
    Handling, PICKUP_DEVIATION_G, PICKUP_SUSTAIN_MS, SHAKE_DEVIATION_G, SHAKE_REQUIRED_TRANSITIONS,
    SHAKE_WINDOW_MS, TILT_SUSTAIN_MS, TILT_Z_THRESHOLD_G,
};
pub use look_at_sound::{
    LISTEN_RELEASE_MS, LISTEN_RMS_THRESHOLD, LISTEN_SUSTAIN_TICKS, LookAtSound,
};
pub use petting::{PETTING_SUSTAIN_TICKS, Petting};
