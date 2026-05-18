//! End-to-end coverage of the desktop wire protocol — parse a
//! realistic fixture for every [`Inbound`] variant, render every
//! [`Outbound`] variant, and verify the [`LineFramer`] drives the
//! parser across fragmented input the way BLE notifications would.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests assert structural invariants; panic / expect / unwrap are the standard test idiom"
)]

use stackchan_desktop_protocol::{
    Ack, BatteryStatus, Cmd, Decision, Inbound, LineFramer, Outbound, StatusData, SysStatus,
    UserStats, parse_inbound, parse_outbound, render_outbound,
};

// ============================================================
// Inbound — Snapshot
// ============================================================

#[test]
fn snapshot_full_payload() {
    let line = br#"{"total":3,"running":1,"waiting":1,"msg":"approve: Bash","entries":["10:42 git push","10:41 yarn test","10:39 reading file..."],"tokens":184502,"tokens_today":31200,"prompt":{"id":"req_abc123","tool":"Bash","hint":"rm -rf /tmp/foo"}}"#;
    let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
        panic!("expected snapshot");
    };
    assert_eq!(snap.total, 3);
    assert_eq!(snap.running, 1);
    assert_eq!(snap.waiting, 1);
    assert_eq!(snap.msg, "approve: Bash");
    assert_eq!(snap.entries.len(), 3);
    assert_eq!(snap.entries[0], "10:42 git push");
    assert_eq!(snap.tokens, 184_502);
    assert_eq!(snap.tokens_today, 31_200);
    let p = snap.prompt.expect("prompt should be set");
    assert_eq!(p.id, "req_abc123");
    assert_eq!(p.tool, "Bash");
    assert_eq!(p.hint, "rm -rf /tmp/foo");
}

#[test]
fn snapshot_keepalive_empty_object() {
    // The spec says a keepalive is "a snapshot with no changes" —
    // an empty object should parse to a default Snapshot.
    let line = br"{}";
    let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
        panic!("expected snapshot");
    };
    assert_eq!(snap.total, 0);
    assert!(snap.entries.is_empty());
    assert!(snap.prompt.is_none());
}

#[test]
fn snapshot_with_null_prompt_means_no_prompt() {
    let line = br#"{"total":1,"running":1,"waiting":0,"prompt":null}"#;
    let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
        panic!("expected snapshot");
    };
    assert!(snap.prompt.is_none());
}

#[test]
fn snapshot_msg_can_be_null() {
    let line = br#"{"total":0,"msg":null}"#;
    let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
        panic!("expected snapshot");
    };
    assert_eq!(snap.msg, "");
}

#[test]
fn snapshot_skips_unknown_fields_for_forward_compat() {
    // A future desktop version adds `mode: "agent"` — must not
    // brick existing firmware.
    let line = br#"{"total":2,"mode":"agent","new_field":42}"#;
    let Inbound::Snapshot(snap) = parse_inbound(line).unwrap() else {
        panic!("expected snapshot");
    };
    assert_eq!(snap.total, 2);
}

// ============================================================
// Inbound — Turn
// ============================================================

#[test]
fn turn_with_text_block() {
    let line =
        br#"{"evt":"turn","role":"assistant","content":[{"type":"text","text":"hello world"}]}"#;
    let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
        panic!("expected turn");
    };
    assert_eq!(turn.role, "assistant");
    assert_eq!(turn.content.len(), 1);
    assert_eq!(turn.content[0].kind, "text");
    assert_eq!(turn.content[0].text.as_deref(), Some("hello world"));
    assert!(turn.content[0].raw_json.contains("\"type\":\"text\""));
}

#[test]
fn turn_with_tool_use_block() {
    let line = br#"{"evt":"turn","role":"assistant","content":[{"type":"tool_use","id":"toolu_x","name":"Bash","input":{"command":"ls"}}]}"#;
    let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
        panic!("expected turn");
    };
    assert_eq!(turn.content[0].kind, "tool_use");
    assert!(turn.content[0].text.is_none());
    // raw_json preserves the structure for downstream rendering.
    assert!(turn.content[0].raw_json.contains("\"name\":\"Bash\""));
    assert!(turn.content[0].raw_json.contains("\"command\":\"ls\""));
}

#[test]
fn turn_role_defaults_to_assistant_when_absent() {
    let line = br#"{"evt":"turn","content":[{"type":"text","text":"x"}]}"#;
    let Inbound::Turn(turn) = parse_inbound(line).unwrap() else {
        panic!("expected turn");
    };
    assert_eq!(turn.role, "assistant");
}

// ============================================================
// Inbound — TimeSync
// ============================================================

#[test]
fn time_sync_array() {
    let line = br#"{"time":[1775731234,-25200]}"#;
    let Inbound::TimeSync {
        epoch_secs,
        tz_offset_secs,
    } = parse_inbound(line).unwrap()
    else {
        panic!("expected timesync");
    };
    assert_eq!(epoch_secs, 1_775_731_234);
    assert_eq!(tz_offset_secs, -25_200);
}

#[test]
fn time_sync_wrong_arity_rejected() {
    let line = br#"{"time":[1775731234]}"#;
    assert!(parse_inbound(line).is_err());
}

// ============================================================
// Inbound — Cmd variants
// ============================================================

#[test]
fn cmd_owner_extracted() {
    let line = br#"{"cmd":"owner","name":"Felix"}"#;
    let Inbound::Cmd(Cmd::Owner { name }) = parse_inbound(line).unwrap() else {
        panic!("expected owner cmd");
    };
    assert_eq!(name, "Felix");
}

#[test]
fn cmd_status_zero_arg() {
    let line = br#"{"cmd":"status"}"#;
    let Inbound::Cmd(Cmd::Status) = parse_inbound(line).unwrap() else {
        panic!("expected status cmd");
    };
}

#[test]
fn cmd_set_name() {
    let line = br#"{"cmd":"name","name":"Clawd"}"#;
    let Inbound::Cmd(Cmd::SetName { name }) = parse_inbound(line).unwrap() else {
        panic!("expected name cmd");
    };
    assert_eq!(name, "Clawd");
}

#[test]
fn cmd_unpair() {
    let line = br#"{"cmd":"unpair"}"#;
    let Inbound::Cmd(Cmd::Unpair) = parse_inbound(line).unwrap() else {
        panic!("expected unpair cmd");
    };
}

#[test]
fn cmd_char_begin() {
    let line = br#"{"cmd":"char_begin","name":"bufo","total":184320}"#;
    let Inbound::Cmd(Cmd::CharBegin { name, total }) = parse_inbound(line).unwrap() else {
        panic!("expected char_begin cmd");
    };
    assert_eq!(name, "bufo");
    assert_eq!(total, 184_320);
}

#[test]
fn cmd_file_announce() {
    let line = br#"{"cmd":"file","path":"manifest.json","size":412}"#;
    let Inbound::Cmd(Cmd::File { path, size }) = parse_inbound(line).unwrap() else {
        panic!("expected file cmd");
    };
    assert_eq!(path, "manifest.json");
    assert_eq!(size, 412);
}

#[test]
fn cmd_chunk_decodes_base64() {
    // "hello" in base64 = aGVsbG8=
    let line = br#"{"cmd":"chunk","d":"aGVsbG8="}"#;
    let Inbound::Cmd(Cmd::Chunk { data }) = parse_inbound(line).unwrap() else {
        panic!("expected chunk cmd");
    };
    assert_eq!(data, b"hello");
}

#[test]
fn cmd_file_end_and_char_end() {
    let Inbound::Cmd(Cmd::FileEnd) = parse_inbound(br#"{"cmd":"file_end"}"#).unwrap() else {
        panic!("expected file_end");
    };
    let Inbound::Cmd(Cmd::CharEnd) = parse_inbound(br#"{"cmd":"char_end"}"#).unwrap() else {
        panic!("expected char_end");
    };
}

#[test]
fn cmd_unknown_returns_unknown_kind() {
    let line = br#"{"cmd":"future_thing","x":1}"#;
    let err = parse_inbound(line).unwrap_err();
    // The exact error variant matters because the firmware will
    // log the unknown kind and continue rather than disconnect.
    let msg = format!("{err}");
    assert!(msg.contains("future_thing"), "got: {msg}");
}

// ============================================================
// Inbound — error cases
// ============================================================

#[test]
fn empty_input_rejected() {
    assert!(parse_inbound(b"").is_err());
}

#[test]
fn non_object_top_level_rejected() {
    assert!(parse_inbound(b"[]").is_err());
    assert!(parse_inbound(b"\"hi\"").is_err());
    assert!(parse_inbound(b"42").is_err());
}

#[test]
fn malformed_json_rejected() {
    assert!(parse_inbound(b"{\"x\":1").is_err()); // missing close
    assert!(parse_inbound(b"{\"x\":}").is_err()); // missing value
    assert!(parse_inbound(b"{x:1}").is_err()); // bare key
}

#[test]
fn invalid_utf8_rejected() {
    assert!(parse_inbound(&[0xff, 0xfe, 0xfd]).is_err());
}

#[test]
fn float_in_int_field_rejected() {
    // The protocol uses integers throughout; a float would silently
    // truncate, which is worse than rejecting.
    assert!(parse_inbound(br#"{"total":1.5}"#).is_err());
}

#[test]
fn snapshot_negative_count_rejected() {
    assert!(parse_inbound(br#"{"total":-1}"#).is_err());
}

#[test]
fn prompt_without_id_rejected() {
    assert!(parse_inbound(br#"{"prompt":{"tool":"Bash"}}"#).is_err());
}

// ============================================================
// Outbound — render
// ============================================================

#[test]
fn permission_decision_once_renders_exact() {
    let msg = Outbound::Permission {
        id: "req_abc123".into(),
        decision: Decision::Once,
    };
    assert_eq!(
        render_outbound(&msg),
        r#"{"cmd":"permission","id":"req_abc123","decision":"once"}"#
    );
}

#[test]
fn permission_decision_deny_renders_exact() {
    let msg = Outbound::Permission {
        id: "req_xyz".into(),
        decision: Decision::Deny,
    };
    assert_eq!(
        render_outbound(&msg),
        r#"{"cmd":"permission","id":"req_xyz","decision":"deny"}"#
    );
}

#[test]
fn ack_minimal() {
    let msg = Outbound::Ack(Ack {
        cmd: "name".into(),
        ok: true,
        n: 0,
        error: None,
    });
    assert_eq!(render_outbound(&msg), r#"{"ack":"name","ok":true,"n":0}"#);
}

#[test]
fn ack_with_error() {
    let msg = Outbound::Ack(Ack {
        cmd: "file".into(),
        ok: false,
        n: 0,
        error: Some("path traversal rejected".into()),
    });
    assert_eq!(
        render_outbound(&msg),
        r#"{"ack":"file","ok":false,"n":0,"error":"path traversal rejected"}"#
    );
}

#[test]
fn ack_with_n_counter() {
    let msg = Outbound::Ack(Ack {
        cmd: "chunk".into(),
        ok: true,
        n: 4096,
        error: None,
    });
    assert_eq!(
        render_outbound(&msg),
        r#"{"ack":"chunk","ok":true,"n":4096}"#
    );
}

#[test]
fn status_ack_full() {
    let msg = Outbound::StatusAck(StatusData {
        name: "Clawd".into(),
        sec: true,
        battery: Some(BatteryStatus {
            pct: 87,
            mv: 4012,
            ma: -120,
            usb: true,
        }),
        sys: Some(SysStatus {
            uptime_secs: 8412,
            heap_free_bytes: 84_200,
        }),
        stats: Some(UserStats {
            approvals: 42,
            denies: 3,
            velocity: 8,
            naps: 12,
            level: 5,
        }),
    });
    // Compare to the literal fixture from REFERENCE.md (key order
    // matches).
    let rendered = render_outbound(&msg);
    assert_eq!(
        rendered,
        r#"{"ack":"status","ok":true,"data":{"name":"Clawd","sec":true,"bat":{"pct":87,"mV":4012,"mA":-120,"usb":true},"sys":{"up":8412,"heap":84200},"stats":{"appr":42,"deny":3,"vel":8,"nap":12,"lvl":5}}}"#
    );
}

#[test]
fn status_ack_minimal_no_subfields() {
    let msg = Outbound::StatusAck(StatusData {
        name: String::new(),
        sec: false,
        battery: None,
        sys: None,
        stats: None,
    });
    assert_eq!(
        render_outbound(&msg),
        r#"{"ack":"status","ok":true,"data":{"sec":false}}"#
    );
}

#[test]
fn string_escapes_in_outbound() {
    // Tool hints carry arbitrary user input — make sure the
    // builder escapes quotes, backslashes, and control characters.
    let msg = Outbound::Ack(Ack {
        cmd: "toast".into(),
        ok: false,
        n: 0,
        error: Some("line1\nline2\twith \"quotes\"".into()),
    });
    let rendered = render_outbound(&msg);
    assert!(rendered.contains(r"\n"));
    assert!(rendered.contains(r"\t"));
    assert!(rendered.contains(r#"\"quotes\""#));
}

// ============================================================
// LineFramer integration
// ============================================================

#[test]
fn framer_drives_parser_across_fragmented_input() {
    // Simulate two BLE notifications carrying one snapshot each,
    // with the second one split across three writes.
    let mut framer = LineFramer::new();
    let mut lines = Vec::new();
    framer.push(b"{\"total\":1}\n{\"total", &mut lines);
    framer.push(b"\":2,\"runn", &mut lines);
    framer.push(b"ing\":1}\n", &mut lines);
    assert_eq!(lines.len(), 2);

    let Inbound::Snapshot(s0) = parse_inbound(&lines[0]).unwrap() else {
        panic!("first should parse");
    };
    assert_eq!(s0.total, 1);

    let Inbound::Snapshot(s1) = parse_inbound(&lines[1]).unwrap() else {
        panic!("second should parse");
    };
    assert_eq!(s1.total, 2);
    assert_eq!(s1.running, 1);
}

// ============================================================
// Round-trip — confirms build.rs output is parseable by parse_outbound
// ============================================================

#[test]
fn round_trip_permission_decisions() {
    for decision in [Decision::Once, Decision::Deny] {
        let original = Outbound::Permission {
            id: "req_round_trip".into(),
            decision,
        };
        let bytes = render_outbound(&original);
        let parsed = parse_outbound(bytes.as_bytes()).unwrap();
        assert_eq!(parsed, original);
    }
}

#[test]
fn round_trip_ack_with_and_without_error() {
    for ack in [
        Ack {
            cmd: "chunk".into(),
            ok: true,
            n: 1024,
            error: None,
        },
        Ack {
            cmd: "file".into(),
            ok: false,
            n: 0,
            error: Some("rejected: path traversal".into()),
        },
    ] {
        let bytes = render_outbound(&Outbound::Ack(ack.clone()));
        let parsed = parse_outbound(bytes.as_bytes()).unwrap();
        assert_eq!(parsed, Outbound::Ack(ack));
    }
}

#[test]
fn round_trip_full_status_ack() {
    let data = StatusData {
        name: "Clawd".into(),
        sec: true,
        battery: Some(BatteryStatus {
            pct: 87,
            mv: 4012,
            ma: -120,
            usb: true,
        }),
        sys: Some(SysStatus {
            uptime_secs: 8412,
            heap_free_bytes: 84_200,
        }),
        stats: Some(UserStats {
            approvals: 42,
            denies: 3,
            velocity: 8,
            naps: 12,
            level: 5,
        }),
    };
    let bytes = render_outbound(&Outbound::StatusAck(data.clone()));
    let parsed = parse_outbound(bytes.as_bytes()).unwrap();
    assert_eq!(parsed, Outbound::StatusAck(data));
}

#[test]
fn framer_recovers_after_oversize_line() {
    let mut framer = LineFramer::new();
    let mut lines = Vec::new();
    // A junk line bigger than the buffer.
    let junk = vec![b'x'; 10_000];
    framer.push(&junk, &mut lines);
    framer.push(b"\n", &mut lines);
    // Now a clean line.
    framer.push(b"{\"total\":7}\n", &mut lines);
    assert_eq!(lines.len(), 1);
    let Inbound::Snapshot(s) = parse_inbound(&lines[0]).unwrap() else {
        panic!();
    };
    assert_eq!(s.total, 7);
}
