//! # stackchan-core
//!
//! `no_std` domain library for the StackChan NPC. Models the entity as
//! a composition of typed sub-components ([`Entity`] = `{ face, motor,
//! perception, voice, mind, events, tick }`) animated by two trait
//! families:
//!
//! - [`Modifier`] — per-frame state mutators (the 14 stock animation
//!   behaviors live here).
//! - [`Skill`] — Claude-Code-Skill-style discoverable capabilities
//!   (trait surface only today; v2.x).
//!
//! Behaviors register with a [`Director`] which sorts modifiers by
//! [`director::Phase`] + priority and runs them each frame. The phase
//! enum encodes the canonical NPC tick order (Perception → Cognition →
//! Affect → Speech → Expression → Motion → Audio → Output).
//!
//! The crate has no hardware, OS, or allocation dependencies — it's the
//! platform-independent heart of the firmware.
//!
//! ## Stability
//!
//! Everything in this crate is **experimental** as of v0.x. See the
//! top-level `STABILITY.md`.
//!
//! ## Example
//!
//! ```
//! use stackchan_core::{Director, Entity, Instant, modifiers::Blink};
//!
//! let mut entity = Entity::default();
//! let mut blink = Blink::new();
//! let mut director = Director::new();
//! director.add_modifier(&mut blink).expect("registry has room");
//!
//! // Advance simulated time; the blink modifier animates the eyes.
//! for ms in (0..10_000).step_by(33) {
//!     director.run(&mut entity, Instant::from_millis(ms));
//! }
//! ```
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

pub mod clock;
pub mod director;
pub mod draw;
pub mod emotion;
pub mod entity;
pub mod events;
pub mod face;
pub mod head;
pub mod input;
pub mod leds;
pub mod mind;
pub mod modifier;
pub mod modifiers;
pub mod motor;
pub mod perception;
pub mod skill;
pub mod voice;

pub use clock::{Clock, Instant};
pub use director::{
    Director, Field, FieldGroup, MODIFIER_CAP, ModifierMeta, Phase, SKILL_CAP, SkillMeta,
};
pub use emotion::Emotion;
pub use entity::{Entity, Tick};
pub use events::Events;
pub use face::{Eye, EyePhase, Face, Mouth, Point, SCALE_DEFAULT, Style};
pub use head::{HeadDriver, MAX_PAN_DEG, MAX_TILT_DEG, MIN_TILT_DEG, Pose};
pub use input::Input;
pub use leds::{BRIGHTNESS_PEAK, LED_COUNT, LedFrame, render_leds};
pub use mind::{Affect, Attention, Autonomy, Intent, Memory, Mind, OverrideSource};
pub use modifier::Modifier;
pub use motor::Motor;
pub use perception::Perception;
pub use skill::{Skill, SkillStatus};
pub use voice::{ChirpKind, Voice};
