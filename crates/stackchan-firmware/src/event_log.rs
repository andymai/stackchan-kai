//! Bounded RAM ring of operator-visible events.
//!
//! Records lifecycle transitions (boot, Wi-Fi up/down, SD mount), control
//! actions (authenticated POSTs), and warnings. The HTTP handler reads
//! recent entries via `GET /events` so an operator can see what the
//! device did without having to attach to defmt-over-USB.
//!
//! Storage: a single critical-section-guarded `[Entry; CAP]` ring — small
//! enough to live in internal SRAM. Volatile by design (drains on reset);
//! persistent panic capture is a separate concern.

use core::fmt::Write as _;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;
use heapless::String;

/// Capacity of the ring.
///
/// 128 slots × ~88 bytes ≈ 11 KiB — tiny enough for internal SRAM.
pub const CAP: usize = 128;

/// Maximum length of an event message.
///
/// 64 bytes is enough for any `format!`-style line the firmware emits today.
pub const MSG_MAX: usize = 64;

/// Event severity / source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Boot, Wi-Fi link transitions, SD mount/unmount, settings PUT.
    Lifecycle,
    /// Authenticated control-plane action (POST /emotion, /look-at, ...).
    Control,
    /// Anything the firmware also emitted at warn/error level.
    Warn,
}

impl Kind {
    /// Wire-format label written into the JSON response.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Control => "control",
            Self::Warn => "warn",
        }
    }
}

/// One ring entry.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Monotonic-clock timestamp at the moment of recording.
    pub at_ms: u64,
    /// Severity / source bucket.
    pub kind: Kind,
    /// Human-readable message, capped at [`MSG_MAX`].
    pub message: String<MSG_MAX>,
}

impl Entry {
    /// All-zero placeholder. Used for ring-buffer initialisation; the
    /// reader skips entries with `at_ms == 0` until the producer
    /// overwrites them.
    const fn empty() -> Self {
        Self {
            at_ms: 0,
            kind: Kind::Lifecycle,
            message: String::new(),
        }
    }
}

/// Ring-buffer state behind a single critical-section mutex.
///
/// Writes are rare (bounded by control actions + lifecycle transitions);
/// reads are rare (HTTP request). Lock contention is a non-issue, so a
/// blocking mutex keeps the call sites cheap and lets the panic handler
/// — when it eventually lands — reach this without going through async.
struct Ring {
    /// Head index — next write position. Wraps at [`CAP`].
    head: usize,
    /// Total writes since boot. Used to compute the iteration start
    /// point and to expose a monotonically-increasing event count.
    total: u64,
    /// Backing storage. Empty entries (`at_ms == 0`) before `total >=
    /// CAP` are skipped by the reader.
    slots: [Entry; CAP],
}

impl Ring {
    /// Construct an empty ring.
    const fn new() -> Self {
        Self {
            head: 0,
            total: 0,
            slots: [const { Entry::empty() }; CAP],
        }
    }
}

/// The single backing ring. Locked via a blocking critical-section
/// mutex; held only across short cell-replace operations.
static RING: Mutex<CriticalSectionRawMutex, core::cell::RefCell<Ring>> =
    Mutex::new(core::cell::RefCell::new(Ring::new()));

/// Record an event. Truncates the message at [`MSG_MAX`] without
/// erroring — operator-visible noise should never panic the firmware.
pub fn record(kind: Kind, message: &str) {
    let mut msg: String<MSG_MAX> = String::new();
    if msg.push_str(message).is_err() {
        // Fall back to a char-by-char copy that respects boundaries.
        msg.clear();
        for ch in message.chars() {
            if msg.push(ch).is_err() {
                break;
            }
        }
    }
    let now = Instant::now().as_millis();
    RING.lock(|cell| {
        let mut ring = cell.borrow_mut();
        let head = ring.head;
        ring.slots[head] = Entry {
            at_ms: now,
            kind,
            message: msg,
        };
        ring.head = (head + 1) % CAP;
        ring.total = ring.total.saturating_add(1);
    });
}

/// Convenience for the common `format!`-style call path.
pub fn record_fmt(kind: Kind, args: core::fmt::Arguments<'_>) {
    let mut buf: String<MSG_MAX> = String::new();
    let _ = buf.write_fmt(args);
    record(kind, &buf);
}

/// Snapshot copy of the most recent up-to-`limit` entries, oldest first.
///
/// Returns the total event count and a `heapless::Vec` so the caller can
/// shape the JSON payload without holding the mutex across the write.
#[must_use]
pub fn drain_recent(limit: usize) -> (u64, heapless::Vec<Entry, CAP>) {
    let take = limit.min(CAP);
    RING.lock(|cell| {
        let ring = cell.borrow();
        let mut out: heapless::Vec<Entry, CAP> = heapless::Vec::new();
        if ring.total == 0 {
            return (0, out);
        }
        // `total` is u64 but always clamps to CAP for indexing — usize
        // suffices and the cast can't truncate after `min(CAP)`.
        let total_usize = usize::try_from(ring.total).unwrap_or(usize::MAX);
        let count = total_usize.min(CAP).min(take);
        // Walk back `count` slots from `head` (exclusive) so the result
        // is oldest-first within the window the caller asked for.
        let start = (ring.head + CAP - count) % CAP;
        for i in 0..count {
            let entry = &ring.slots[(start + i) % CAP];
            // Bounded `heapless::Vec` — the `count.min(CAP).min(take)`
            // upper bound guarantees push always succeeds.
            let _ = out.push(entry.clone());
        }
        (ring.total, out)
    })
}
