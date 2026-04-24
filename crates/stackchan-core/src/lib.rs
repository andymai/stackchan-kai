//! # stackchan-core
//!
//! `no_std` domain library for the StackChan avatar. Models the face as data
//! and drives animation through a [`Modifier`] trait that mutates an
//! [`Avatar`] in response to the passage of time (supplied by a [`Clock`]).
//!
//! The crate has no hardware, OS, or allocation dependencies -- it's the
//! platform-independent heart of the firmware.
//!
//! ## Stability
//!
//! Everything in this crate is **experimental** as of v0.1.0. See the
//! top-level `STABILITY.md`.
//!
//! ## Example
//!
//! ```
//! use stackchan_core::{Avatar, Instant, Modifier, modifiers::Blink};
//!
//! let mut avatar = Avatar::default();
//! let mut blink = Blink::new();
//!
//! // Advance simulated time; the blink modifier animates the eyes.
//! for ms in 0..10_000 {
//!     let now = Instant::from_millis(ms);
//!     blink.update(&mut avatar, now);
//! }
//! ```
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

pub mod avatar;
pub mod clock;
pub mod draw;
pub mod emotion;
pub mod modifiers;

pub use avatar::{Avatar, Eye, EyePhase, Mouth, Point, SCALE_DEFAULT};
pub use clock::{Clock, Instant};
pub use emotion::Emotion;
pub use modifiers::Modifier;
