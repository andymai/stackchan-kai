//! # stackchan-tts
//!
//! Speech synthesis abstractions for the Stack-chan firmware.
//!
//! ## Layers
//!
//! - [`AudioSource`] — pull-based PCM producer. Implementors include
//!   sine-cycle SFX (current chirps), baked-PCM clips (verbal phrases
//!   embedded via `include_bytes!`), and streamed cloud chunks.
//! - [`SpeechBackend`] — resolves an [`Utterance`] (from
//!   `stackchan-core::voice`) into an [`AudioSource`]. The firmware
//!   speech router holds one or more backends and dispatches by
//!   [`SpeechContent`] kind.
//! - [`LipSync`] — per-tick envelope + optional viseme published by the
//!   audio task while a source plays. Drives the avatar's mouth during
//!   self-speech. Lives in `stackchan-core::lipsync` (re-exported here)
//!   so the `Perception` layer can carry it as a per-frame field.
//!
//! Domain types — [`Utterance`], [`PhraseId`], [`Locale`],
//! [`SpeechContent`], [`SpeechStyle`], [`Priority`], [`ContentRef`] —
//! live in `stackchan-core::voice` so modifiers can publish them
//! without depending on this crate.
//!
//! ## Stability
//!
//! Experimental as of v0.1.0; API surface will move as backends land.
//!
//! [`Utterance`]: stackchan_core::voice::Utterance
//! [`PhraseId`]: stackchan_core::voice::PhraseId
//! [`Locale`]: stackchan_core::voice::Locale
//! [`SpeechContent`]: stackchan_core::voice::SpeechContent
//! [`SpeechStyle`]: stackchan_core::voice::SpeechStyle
//! [`Priority`]: stackchan_core::voice::Priority
//! [`ContentRef`]: stackchan_core::voice::ContentRef

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

pub mod backend;
pub mod baked;
pub mod source;

pub use backend::{RenderError, SpeechBackend};
pub use baked::{BakedBackend, SineSequence, SineTableSource};
pub use source::AudioSource;
// Re-export lip-sync types from core; they live there because the
// `Perception` layer carries them as a per-frame field.
pub use stackchan_core::lipsync::{LipSync, Viseme};
