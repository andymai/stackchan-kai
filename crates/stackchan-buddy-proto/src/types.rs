//! Wire-format message types.

use alloc::string::String;
use alloc::vec::Vec;

/// Anything the desktop sends the device on the RX characteristic.
///
/// Each variant is a complete JSON object that arrived on its own
/// line. [`crate::parse_inbound`] dispatches by inspecting the
/// top-level keys: `cmd` → [`Inbound::Cmd`], `evt` →
/// [`Inbound::Turn`], `time` → [`Inbound::TimeSync`], otherwise
/// [`Inbound::Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// Periodic session-state snapshot. Sent on change and at
    /// least every 10 s as a keepalive.
    Snapshot(Snapshot),

    /// One-shot turn event: the raw content array of an assistant
    /// message that just completed.
    Turn(Turn),

    /// One-shot time sync. `epoch_secs` is Unix seconds;
    /// `tz_offset_secs` is the local timezone offset east of UTC.
    TimeSync {
        /// Unix epoch seconds. May be negative in principle; will
        /// never be in practice.
        epoch_secs: i64,
        /// Local timezone offset east of UTC, in seconds.
        tz_offset_secs: i32,
    },

    /// A `cmd`-tagged message that's either a one-shot piece of
    /// state (`owner`) or a request the device should ack.
    Cmd(Cmd),
}

/// Heartbeat snapshot — the desktop's running view of itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Count of all sessions.
    pub total: u32,
    /// Sessions actively generating.
    pub running: u32,
    /// Sessions blocked on a permission prompt.
    pub waiting: u32,
    /// One-line summary suitable for a small display.
    pub msg: String,
    /// Recent transcript lines, newest first.
    pub entries: Vec<String>,
    /// Cumulative output tokens since the desktop app started.
    pub tokens: u64,
    /// Output tokens since local midnight. Persisted by the
    /// desktop across restarts.
    pub tokens_today: u64,
    /// Pending permission decision, if one is blocking a session.
    pub prompt: Option<Prompt>,
}

/// A permission prompt the desktop is asking the device to decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Opaque request ID the device echoes back in its decision.
    pub id: String,
    /// Tool name (e.g. `"Bash"`, `"Edit"`).
    pub tool: String,
    /// One-line hint about the specific call (the bash command,
    /// the file being edited, etc.). May be empty.
    pub hint: String,
}

/// Completed-turn event. Mirrors the SDK content array for the
/// assistant turn that just finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Message role. The spec uses `"assistant"` today; preserved
    /// as a string for forward compatibility.
    pub role: String,
    /// Heterogeneous content blocks: text + tool calls + anything
    /// future versions add.
    pub content: Vec<ContentBlock>,
}

/// One element of a [`Turn::content`] array.
///
/// The protocol mirrors the upstream SDK schema, where each block
/// has a `"type"` discriminator and arbitrary additional fields.
/// We keep the discriminator + the most commonly used field
/// (`text` for `"text"` blocks) and store the raw JSON object for
/// everything else so consumers can route on `kind` without paying
/// to model every variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlock {
    /// `type` field from the wire.
    pub kind: String,
    /// `text` field, if present (set for `kind == "text"`).
    pub text: Option<String>,
    /// Compact JSON re-rendering of the block, suitable for
    /// downstream forwarding or rendering a tool-call summary.
    /// Object key order matches the wire; whitespace and number
    /// formatting may differ. Not byte-identical to the input
    /// — use this for routing on `kind`, not for cryptographic
    /// round-trip.
    pub raw_json: String,
}

/// Desktop → device commands. Anything with a top-level `cmd`
/// field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// `{"cmd":"owner","name":"Felix"}` — one-shot on connect.
    /// Sets the user's first name on the device.
    Owner {
        /// User's first name, as configured in the desktop app.
        name: String,
    },

    /// `{"cmd":"status"}` — desktop polls every couple of
    /// seconds; expects a [`Outbound::StatusAck`] in reply.
    Status,

    /// `{"cmd":"name","name":"Clawd"}` — sets the device's
    /// display name. Expects a generic [`Outbound::Ack`].
    SetName {
        /// New display name.
        name: String,
    },

    /// `{"cmd":"unpair"}` — erase stored BLE bonds.
    Unpair,

    /// `{"cmd":"char_begin","name":"bufo","total":184320}` —
    /// start of a folder push. `total` is the sum of file sizes in
    /// bytes (transport overhead not included).
    CharBegin {
        /// Folder name (derived from the dropped folder, or
        /// overridden by a `manifest.json.name` field).
        name: String,
        /// Sum of file sizes that will be pushed, in bytes.
        total: u32,
    },

    /// `{"cmd":"file","path":"manifest.json","size":412}` — start
    /// of one file within a folder push.
    File {
        /// Relative path inside the folder. Always validate via
        /// [`is_safe_relative_path`] before joining onto a writable
        /// root — the desktop sends whatever filenames appeared in
        /// the dropped folder, including traversal attempts from a
        /// malicious central.
        path: String,
        /// File size in bytes.
        size: u32,
    },

    /// `{"cmd":"chunk","d":"<base64>"}` — one base64-encoded
    /// chunk of file bytes. The protocol is sequential: each chunk
    /// belongs to the file announced by the most recent
    /// [`Cmd::File`].
    Chunk {
        /// Decoded chunk bytes.
        data: Vec<u8>,
    },

    /// `{"cmd":"file_end"}` — end of the current file.
    FileEnd,

    /// `{"cmd":"char_end"}` — end of the folder push.
    CharEnd,
}

/// Anything the device sends back on the TX characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// Reply to a [`Prompt`]. `id` must equal the prompt's id
    /// exactly. The spec accepts `"once"` (approve this single
    /// invocation) and `"deny"` only.
    Permission {
        /// Echoed `prompt.id`.
        id: String,
        /// Decision variant.
        decision: Decision,
    },

    /// Generic ack for any `cmd`-tagged command other than
    /// `status`. `n` is a counter (e.g. bytes-written-so-far for
    /// chunk acks); set to 0 when not meaningful.
    Ack(Ack),

    /// Ack for `{"cmd":"status"}` with the device's reported
    /// data payload.
    StatusAck(StatusData),
}

/// Permission decision variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Approve this single tool call.
    Once,
    /// Reject this tool call.
    Deny,
}

impl Decision {
    /// Wire representation (the value of the `"decision"` field).
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Deny => "deny",
        }
    }
}

/// Body of a generic [`Outbound::Ack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ack {
    /// Echo of the inbound `cmd` field that this acks.
    pub cmd: String,
    /// Whether the command was honored.
    pub ok: bool,
    /// Counter — bytes written so far for `chunk`/`file_end`,
    /// otherwise 0.
    pub n: u32,
    /// Optional human-readable error string (only emit when `ok`
    /// is `false`).
    pub error: Option<String>,
}

/// Body of a status ack — the desktop polls this every couple of
/// seconds to populate its stats panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusData {
    /// Device display name. Omitted on the wire when empty.
    pub name: String,
    /// `true` if the BLE link is encrypted (bonded). Encoded
    /// verbatim in the ack; the desktop reads it for the lock
    /// indicator.
    pub sec: bool,
    /// Battery telemetry, optional.
    pub battery: Option<BatteryStatus>,
    /// System telemetry, optional.
    pub sys: Option<SysStatus>,
    /// User-facing counters, optional.
    pub stats: Option<UserStats>,
}

/// Battery telemetry sub-object of [`StatusData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Charge percent, 0..=100.
    pub pct: u8,
    /// Battery voltage in millivolts.
    pub mv: u16,
    /// Battery current in milliamps. Negative when charging.
    pub ma: i16,
    /// `true` when USB power is attached.
    pub usb: bool,
}

/// System telemetry sub-object of [`StatusData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysStatus {
    /// Uptime in seconds.
    pub uptime_secs: u32,
    /// Free heap bytes.
    pub heap_free_bytes: u32,
}

/// User-facing counter sub-object of [`StatusData`]. Mirrors the
/// keys in the reference spec verbatim so the desktop's stats panel
/// renders without translation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserStats {
    /// `appr` — cumulative approves served from the device.
    pub approvals: u32,
    /// `deny` — cumulative denies served from the device.
    pub denies: u32,
    /// `vel` — derived "velocity" counter (approvals per minute,
    /// EWMA, etc. — the device gets to define what it shows).
    pub velocity: u32,
    /// `nap` — naps / sleep cycles.
    pub naps: u32,
    /// `lvl` — buddy "level" (a free-form gamification counter).
    pub level: u32,
}

/// Validate a [`Cmd::File`] path before joining it onto a
/// writable root.
///
/// Returns `false` for empty input, absolute paths (Unix or
/// Windows), backslash separators, `..` segments, lone `.`
/// segments, leading `~`, embedded NUL, and Windows drive prefixes
/// like `C:`. Conservative by design: the folder-push protocol is
/// meant to carry filenames from a single dropped folder with no
/// recursion, so anything more exotic is suspicious and worth
/// rejecting at the edge rather than after a write has hit disk.
///
/// Returns `true` for the safe path, ready to be appended to a
/// trusted base directory.
#[must_use]
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') || path.starts_with('\\') || path.starts_with('~') {
        return false;
    }
    if path.contains('\\') || path.contains('\0') {
        return false;
    }
    // Reject Windows drive prefixes (`C:`, `c:\path`, etc.) even
    // though we don't run on Windows — the desktop is cross-platform
    // and the constraint is "no absolute paths in any form".
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    // Reject any `..` or `.` segment. Split on `/` rather than
    // walking codepoint-by-codepoint because the protocol is
    // POSIX-style.
    for segment in path.split('/') {
        if segment.is_empty() || segment == ".." || segment == "." {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap are the standard test idiom"
)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths_accepted() {
        for path in [
            "manifest.json",
            "idle_0.gif",
            "subdir/icon.png",
            "deep/nested/path.bin",
            "file with spaces.txt",
            "unicode_名前.dat",
        ] {
            assert!(is_safe_relative_path(path), "should accept: {path:?}");
        }
    }

    #[test]
    fn traversal_segments_rejected() {
        for path in [
            "..",
            "../etc/passwd",
            "foo/..",
            "foo/../bar",
            "a/b/../../c",
            ".",
            "./hidden",
        ] {
            assert!(!is_safe_relative_path(path), "should reject: {path:?}");
        }
    }

    #[test]
    fn absolute_paths_rejected() {
        for path in [
            "/etc/passwd",
            "/",
            "\\Windows\\System32",
            "\\",
            "C:/Users/admin/file",
            "c:\\foo",
            "~/private",
            "~root/.ssh/id_rsa",
        ] {
            assert!(!is_safe_relative_path(path), "should reject: {path:?}");
        }
    }

    #[test]
    fn empty_and_null_rejected() {
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("foo\0bar"));
    }

    #[test]
    fn backslash_separator_rejected() {
        // Windows-style separators are rejected even when the path
        // doesn't otherwise look absolute, because joining with a
        // POSIX base later would not interpret them as separators
        // and a malicious filename could slip in literal backslashes.
        assert!(!is_safe_relative_path("foo\\bar"));
    }
}
