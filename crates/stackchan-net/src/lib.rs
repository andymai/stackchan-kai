//! # stackchan-net
//!
//! Networking domain types for the Stack-chan firmware. Pure data
//! and parsers — no transport, no I/O, no esp-hal. The firmware does
//! the I/O wrapping; this crate is what the firmware (and host tests)
//! agree on as the shape of the on-disk config and the RON file
//! [`PUT /settings`] round-trips.
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
//! [`Config::default`]. The validators in [`parse_ron`] reject
//! malformed input, but the firmware never propagates a
//! [`ConfigError`] up to a panic — it logs and uses defaults.
//!
//! [`PUT /settings`]: # "see crates/stackchan-firmware/src/net/http.rs once it lands"

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

pub mod config;
pub mod error;

pub use config::{Config, MdnsConfig, TimeConfig, WifiConfig, parse_ron, render_ron};
pub use error::ConfigError;
