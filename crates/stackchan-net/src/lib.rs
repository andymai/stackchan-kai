//! # stackchan-net
//!
//! Networking domain types for the Stack-chan firmware. Pure data
//! and parsers — no transport, no I/O, no esp-hal. The firmware does
//! the I/O wrapping; this crate is what the firmware (and host tests)
//! agree on as the shape of the on-disk config and the RON over
//! the wire.
//!
//! ## Schema v1
//!
//! ```ron
//! (
//!     wifi: ( ssid: "home", psk: "redacted", country: "US" ),
//!     mdns: ( hostname: "stackchan" ),
//!     time: ( tz: "UTC", sntp_servers: ["pool.ntp.org"] ),
//! )
//! ```
//!
//! - [`WifiConfig`] — credentials + regulatory country code (default `"US"`).
//! - [`MdnsConfig`] — hostname advertised on `.local` (default `"stackchan"`).
//! - [`TimeConfig`] — timezone label + SNTP servers (default `"UTC"`,
//!   `["pool.ntp.org"]`). The TZ field is parsed but currently unused;
//!   the BM8563 RTC stores UTC.
//!
//! ## Offline-first stance
//!
//! The avatar must boot fully and animate even with no SD card and
//! no Wi-Fi. The firmware therefore treats this crate's [`Config`]
//! as **always available**: missing SD or missing file falls back to
//! [`Config::default`]. Validators reject malformed input but the
//! firmware never propagates a [`ConfigError`] up to a panic — it
//! logs and uses defaults.
//!
//! ## Feature: `parse`
//!
//! Default-on for host builds. Adds:
//! - serde derives on [`Config`] / [`WifiConfig`] / [`MdnsConfig`] / [`TimeConfig`]
//! - [`parse_ron`] / [`render_ron`] (lossless round trip via `ron 0.10`)
//!
//! Disabled for the firmware target because `ron 0.10` hard-pins
//! `serde/std` + `base64/std` — both broken on
//! `xtensa-esp32s3-none-elf` where `std` is absent. The firmware uses
//! its own hand-rolled RON parser and reuses [`validate`] from this
//! crate to enforce the same schema gate the host path runs.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

pub mod bare;
pub mod bare_json;
pub mod config;
pub mod error;
pub mod http_parse;

pub use bare::{parse_ron_bare, render_ron_bare};
pub use bare_json::{merge_settings_with_current, parse_settings_json, render_settings_json};
pub use config::{AuthConfig, Config, MdnsConfig, TimeConfig, WifiConfig, validate};
#[cfg(feature = "parse")]
pub use config::{parse_ron, render_ron};
pub use error::ConfigError;
