//! # stackchan-core
//!
//! `no_std` engine for the StackChan NPC. An [`Entity`] holds the state
//! (face, motor, perception, voice, mind, events, input, tick); a
//! [`Director`] sorts [`Modifier`]s by [`director::Phase`] + priority
//! and ticks them each frame.
//!
//! [`Skill`] is a longer-running NPC capability with `name` +
//! `description` metadata shaped for a future dispatcher.
//!
//! No hardware, OS, or allocation dependencies.
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
pub mod skills;
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
pub use mind::{Affect, Attention, Autonomy, Intent, Mind, OverrideSource};
pub use modifier::Modifier;
pub use motor::Motor;
pub use perception::{BodyTouch, Perception};
pub use skill::{Skill, SkillStatus};
pub use voice::{ChirpKind, Voice};
