//! Claude Desktop Hardware Buddy plumbing.
//!
//! [`DESKTOP_INBOUND`] fans parsed [`Inbound`] messages out to the
//! firmware-side consumers that act on them — face / toast / LED
//! ring, permission decision UX, and the folder-push / status /
//! owner / time-sync handlers. [`DESKTOP_OUTBOUND`] is the
//! single-slot reply channel a consumer signals to send a
//! permission decision or ack back to the desktop.
//!
//! [`DesktopSession`] holds the per-connection [`LineFramer`] plus a
//! few diagnostic counters. The GATT layer creates one per
//! connection in `super::server::gatt_events_task`, feeds inbound
//! bytes through [`DesktopSession::ingest`], and drops it on
//! disconnect — the framer's partial line never leaks across
//! connections.

use alloc::vec::Vec;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_sync::signal::Signal;
use stackchan_desktop_protocol::{Inbound, LineFramer, Outbound, ProtoError, parse_inbound};

/// `PubSub` channel fan-out for inbound desktop messages.
///
/// Capacity 4 absorbs the desktop's burst pattern on connect
/// (snapshot + owner + time-sync arrive back-to-back). 4 subscriber
/// slots cover the face / toast / LED renderer, the permission
/// decision UX, the folder-push writer, and a future hook.
/// Each subscriber receives the same `Inbound` clone — cheap given
/// the ≤10 Hz heartbeat rate and the firmware's PSRAM heap.
pub static DESKTOP_INBOUND: PubSubChannel<CriticalSectionRawMutex, Inbound, 4, 4, 1> =
    PubSubChannel::new();

/// Single-slot outbound queue.
///
/// Producers (permission UX, folder-push acks, status ack) call
/// [`Signal::signal`]; the BLE serve loop drains via
/// [`Signal::wait`] and notifies the NUS TX characteristic.
/// Latest-wins semantics are acceptable because the desktop only
/// has one outstanding prompt at a time — a second signal before
/// the first is consumed means the operator changed their decision
/// faster than the BLE link could carry it.
pub static DESKTOP_OUTBOUND: Signal<CriticalSectionRawMutex, Outbound> = Signal::new();

/// Per-connection state for the desktop NUS service.
pub struct DesktopSession {
    /// Line accumulator for the NUS RX characteristic. Each ATT
    /// write may carry a fragment, a complete line, or several
    /// lines back-to-back — the framer yields whole `\n`-terminated
    /// objects.
    framer: LineFramer,
    /// Count of lines that failed to parse since the connection
    /// opened. Surfaced in defmt only; the parser logs the specific
    /// error and continues.
    parse_failures: u32,
}

impl Default for DesktopSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopSession {
    /// Empty session — no buffered bytes, no parse failures yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            framer: LineFramer::new(),
            parse_failures: 0,
        }
    }

    /// Absorb one ATT-write payload, drain any newly complete lines
    /// through the parser, and publish each parsed [`Inbound`] onto
    /// [`DESKTOP_INBOUND`].
    ///
    /// Parse failures are counted in `Self::parse_failures` and
    /// logged at `warn` level. The connection is NOT torn down on
    /// parse failure: per spec the desktop is forward-compatible
    /// authority over the schema, and the right firmware response
    /// to an unrecognised line is to skip it.
    pub fn ingest(&mut self, bytes: &[u8]) {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        self.framer.push(bytes, &mut lines);
        if lines.is_empty() {
            return;
        }
        // Take + drop the publisher slot once per ATT write rather
        // than per parsed message. The pubsub is declared with one
        // publisher slot; per-line acquisition would still work via
        // immediate drop, but coalescing avoids the slot churn on
        // the common case of many lines arriving in one write.
        let Ok(publisher) = DESKTOP_INBOUND.publisher() else {
            defmt::warn!(
                "desktop: DESKTOP_INBOUND publisher slot exhausted; dropping {=usize} line(s)",
                lines.len()
            );
            return;
        };
        for line in lines {
            match parse_inbound(&line) {
                Ok(message) => publisher.publish_immediate(message),
                Err(err) => {
                    self.parse_failures = self.parse_failures.saturating_add(1);
                    defmt::warn!(
                        "desktop: parse failed ({}, total={=u32})",
                        log_proto_error(&err),
                        self.parse_failures,
                    );
                }
            }
        }
    }

    /// Drop any partially accumulated bytes. Called on disconnect
    /// so a half-line doesn't poison the next central's session
    /// (`DesktopSession` is per-connection, but the static can't
    /// model that without a Cell — this method is the explicit
    /// reset path).
    pub fn reset(&mut self) {
        self.framer.reset();
        self.parse_failures = 0;
    }
}

/// Map [`ProtoError`] to a short defmt string so we don't reach
/// for `Debug2Format` on every parse miss.
const fn log_proto_error(err: &ProtoError) -> &'static str {
    match err {
        ProtoError::InvalidUtf8 => "invalid utf-8",
        ProtoError::MalformedJson(_) => "malformed json",
        ProtoError::MissingField(_) => "missing field",
        ProtoError::BadValue { .. } => "bad value",
        ProtoError::UnknownKind(_) => "unknown kind",
        ProtoError::InvalidBase64 => "invalid base64",
    }
}
