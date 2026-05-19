//! [`Outbound`] → JSON line.
//!
//! Builders are infallible and emit a single object with no
//! trailing newline (the BLE write layer adds `\n` per line). Order
//! of keys matches the reference spec so on-wire fixtures diff
//! cleanly against the desktop's parser output.

use alloc::string::String;
use core::fmt::Write as _;

use crate::types::{Ack, BatteryStatus, Outbound, StatusData, SysStatus, UserStats};

/// Serialize an outbound message into a single JSON object line.
///
/// Never fails; the type system guarantees a well-formed object.
#[must_use]
pub fn render_outbound(msg: &Outbound) -> String {
    let mut out = String::new();
    match msg {
        Outbound::Permission { id, decision } => {
            out.push_str(r#"{"cmd":"permission","id":"#);
            push_string(&mut out, id);
            out.push_str(r#","decision":""#);
            out.push_str(decision.as_wire());
            out.push_str("\"}");
        }
        Outbound::Ack(ack) => render_ack(&mut out, ack),
        Outbound::StatusAck(data) => render_status_ack(&mut out, data),
    }
    out
}

/// Emit a generic ack: `{"ack":"<cmd>","ok":<bool>,"n":<u32>}`
/// with `"error":"..."` appended only when set.
fn render_ack(out: &mut String, ack: &Ack) {
    out.push_str(r#"{"ack":"#);
    push_string(out, &ack.cmd);
    out.push_str(r#","ok":"#);
    out.push_str(if ack.ok { "true" } else { "false" });
    out.push_str(r#","n":"#);
    // write! to a String can't fail; the unwrap is unreachable.
    let _ = write!(out, "{}", ack.n);
    if let Some(err) = &ack.error {
        out.push_str(r#","error":"#);
        push_string(out, err);
    }
    out.push('}');
}

/// Emit a status ack: `{"ack":"status","ok":true,"data":{...}}`.
/// Empty / `None` sub-fields are omitted so the desktop sees only
/// what the device actually tracks.
fn render_status_ack(out: &mut String, data: &StatusData) {
    out.push_str(r#"{"ack":"status","ok":true,"data":{"#);
    let mut first = true;
    if !data.name.is_empty() {
        push_key(out, &mut first, "name");
        push_string(out, &data.name);
    }
    push_key(out, &mut first, "sec");
    out.push_str(if data.sec { "true" } else { "false" });
    if let Some(bat) = data.battery {
        push_key(out, &mut first, "bat");
        render_battery(out, bat);
    }
    if let Some(sys) = data.sys {
        push_key(out, &mut first, "sys");
        render_sys(out, sys);
    }
    if let Some(stats) = data.stats {
        push_key(out, &mut first, "stats");
        render_stats(out, stats);
    }
    out.push_str("}}");
}

/// Emit the `bat` sub-object of a status ack. Field names match
/// the reference spec verbatim — `mV` and `mA` keep their case.
fn render_battery(out: &mut String, bat: BatteryStatus) {
    let _ = write!(
        out,
        r#"{{"pct":{},"mV":{},"mA":{},"usb":{}}}"#,
        bat.pct,
        bat.mv,
        bat.ma,
        if bat.usb { "true" } else { "false" }
    );
}

/// Emit the `sys` sub-object of a status ack.
fn render_sys(out: &mut String, sys: SysStatus) {
    let _ = write!(
        out,
        r#"{{"up":{},"heap":{}}}"#,
        sys.uptime_secs, sys.heap_free_bytes
    );
}

/// Emit the `stats` sub-object of a status ack. All counters
/// emit unconditionally (zero is a meaningful value).
fn render_stats(out: &mut String, stats: UserStats) {
    let _ = write!(
        out,
        r#"{{"appr":{},"deny":{},"vel":{},"nap":{},"lvl":{}}}"#,
        stats.approvals, stats.denies, stats.velocity, stats.naps, stats.level
    );
}

/// Append `"key":` to `out`, prefixing a comma when `first` is
/// `false`. Flips `first` to `false` after the first call so the
/// caller doesn't have to track separator state.
fn push_key(out: &mut String, first: &mut bool, key: &str) {
    if *first {
        *first = false;
    } else {
        out.push(',');
    }
    out.push('"');
    out.push_str(key);
    out.push_str(r#"":"#);
}

/// Append a JSON string literal (with required escapes) to `out`.
/// Symmetric with [`crate::parse`]'s `write_string`; duplicated
/// here to keep this module dependency-free.
fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: structural pins on the rendered output"
)]
mod tests {
    use super::*;
    use crate::types::Ack;

    #[test]
    fn push_string_escapes_backslash_and_returns_and_control_chars() {
        // Build an Ack with an error string that triggers every
        // push_string escape arm: backslash, `\n` (already covered),
        // `\r`, `\t` (already covered), and a control char like
        // 0x01 routed through `\\u{:04x}`.
        let ack = Ack {
            cmd: alloc::string::String::from("file"),
            ok: false,
            n: 0,
            error: Some(alloc::string::String::from(
                "back\\slash and \r return and \x01 ctrl",
            )),
        };
        let mut out = alloc::string::String::new();
        render_ack(&mut out, &ack);
        // backslash escape — one source backslash → two on the wire.
        assert!(out.contains("back\\\\slash"), "rendered = {out}");
        // \r → the two-char `\r` JSON escape.
        assert!(out.contains("\\r"), "rendered = {out}");
        // 0x01 control char emits the \uXXXX form.
        assert!(out.contains("\\u0001"), "rendered = {out}");
    }

    #[test]
    fn push_string_passes_through_printable_ascii_and_non_ascii() {
        // Non-special / non-control chars take the `c => out.push(c)`
        // fall-through. Pin printable ASCII + non-ASCII codepoint
        // (the latter has u32 ≥ 0x20 so it doesn't hit the control
        // arm).
        let ack = Ack {
            cmd: alloc::string::String::from("name"),
            ok: true,
            n: 0,
            error: Some(alloc::string::String::from("hello 日本")),
        };
        let mut out = alloc::string::String::new();
        render_ack(&mut out, &ack);
        assert!(out.contains("hello 日本"), "rendered = {out}");
    }
}
