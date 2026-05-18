//! # stackchan-buddy-proto
//!
//! Wire protocol for the Claude Desktop Hardware Buddy BLE bridge.
//! Newline-delimited UTF-8 JSON over the Nordic UART Service, one
//! object per line. The desktop apps stream session state and
//! permission prompts; the device replies with permission decisions
//! and command acks.
//!
//! Reference spec:
//! <https://github.com/anthropics/claude-desktop-buddy/blob/main/REFERENCE.md>
//!
//! ## Modules
//!
//! - [`types`] — message types ([`Inbound`], [`Outbound`], [`Snapshot`],
//!   [`Cmd`], [`Ack`], ...).
//! - [`parse`] — bytes → [`Inbound`].
//! - [`build`] — [`Outbound`] → bytes.
//! - [`frame`] — [`LineFramer`] accumulates fragmented BLE
//!   notifications into complete lines.
//! - [`error`] — typed errors.
//!
//! ## Hand-rolled JSON
//!
//! Same rationale as the rest of the workspace's wire-format
//! parsers: the firmware target (`xtensa-esp32s3-none-elf`) can't
//! pull `serde/std`. Inbound parsers skip unknown keys for forward
//! compatibility; outbound builders emit only named fields.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

pub mod build;
pub mod error;
pub mod frame;
pub mod parse;
pub mod types;

pub use build::render_outbound;
pub use error::ProtoError;
pub use frame::LineFramer;
pub use parse::{parse_inbound, parse_outbound};
pub use types::{
    Ack, BatteryStatus, Cmd, ContentBlock, Decision, Inbound, Outbound, Prompt, Snapshot,
    StatusData, SysStatus, Turn, UserStats,
};

/// Nordic UART Service base UUID (`6e400001-b5a3-f393-e0a9-e50e24dcca9e`).
///
/// The desktop's BLE scanner filters on the advertised name prefix
/// `Claude`, but a device that advertises this service UUID is
/// canonical — keep both for compatibility.
pub const NUS_SERVICE_UUID: &str = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";

/// NUS RX characteristic (desktop → device, write or write-without-response).
pub const NUS_RX_CHAR_UUID: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";

/// NUS TX characteristic (device → desktop, notify).
pub const NUS_TX_CHAR_UUID: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";

/// Heartbeat interval the desktop guarantees when paired.
///
/// The spec says "if you don't receive a snapshot for ~30 seconds,
/// treat the connection as dead." Consumers can use this constant
/// to drive a watchdog timer.
pub const HEARTBEAT_GRACE_SECS: u32 = 30;

/// Maximum serialized turn event size in UTF-8 bytes. Events
/// larger than this are dropped by the desktop and never reach the
/// device, so parsers don't need to handle them.
pub const TURN_EVENT_MAX_BYTES: usize = 4096;

/// Maximum folder-push payload size in bytes. Anything larger
/// won't be sent by the desktop's drop target.
pub const FOLDER_PUSH_MAX_BYTES: usize = 1_843_200;
