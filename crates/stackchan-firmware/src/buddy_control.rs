//! Buddy command-surface handler: status / owner / name / unpair
//! / time-sync / turn events / folder push.
//!
//! Subscribes to [`crate::ble::buddy::BUDDY_INBOUND`] for the
//! `cmd`- and `evt`-tagged messages the other buddy tasks
//! ([`crate::buddy_render`], [`crate::buddy_permission`]) don't
//! consume themselves, and signals replies on
//! [`crate::ble::buddy::BUDDY_OUTBOUND`].
//!
//! - **status** → builds a [`StatusData`] from the current
//!   battery / uptime / heap / approval+deny counters and replies.
//! - **owner** → logs + acks.
//! - **name** → logs + acks. Persistence to `/sd/RUNTIME.RON`
//!   joins a later slice; the in-RAM name visible over BLE doesn't
//!   change until that lands.
//! - **unpair** → wipes the SD-backed bonds via
//!   [`crate::ble::bonds::save_all`] with an empty list, then acks.
//! - **time** (`{"time":[epoch,tz]}`) → logs. The BM8563 RTC is
//!   driven by [`crate::wallclock`]'s SNTP path; buddy time is
//!   advisory until a follow-up slice wires it in.
//! - **turn** events → push the assistant's first text block into
//!   the toast band so the operator sees the recent reply.
//! - **`char_begin` / `file` / `chunk` / `file_end` / `char_end`** →
//!   acknowledged with `ok:false` so the desktop's drop target
//!   surfaces a clear failure. The SD writer lands as a follow-up.

use alloc::string::ToString;
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Instant};
use heapless_09::Vec as HVec;
use stackchan_buddy_proto::{
    Ack, BatteryStatus, Cmd, ContentBlock, Inbound, Outbound, StatusData, SysStatus, Turn,
};
use stackchan_core::Clock;

use crate::ble::bonds;
use crate::ble::buddy::{BUDDY_INBOUND, BUDDY_OUTBOUND};
use crate::clock::HalClock;
use crate::net::snapshot;
use crate::toast::{self, ToastLevel};

/// Cap on how much of an assistant text block we push to the toast
/// band. The toast itself truncates by codepoints, but capping
/// before push avoids re-walking long assistant essays on every
/// turn event.
const TOAST_PREVIEW_BYTES: usize = 64;

/// Render task. Spawns once at boot, runs for the firmware
/// lifetime.
#[embassy_executor::task]
pub async fn buddy_control_task() -> ! {
    let Ok(mut sub) = BUDDY_INBOUND.subscriber() else {
        defmt::error!(
            "buddy_control: BUDDY_INBOUND subscriber slot exhausted; task parking forever"
        );
        loop {
            embassy_time::Timer::after(Duration::from_secs(3600)).await;
        }
    };
    let boot = Instant::now();
    defmt::info!("buddy_control: armed (status / owner / name / unpair / turn / push)");

    loop {
        let message = match sub.next_message().await {
            WaitResult::Message(m) => m,
            WaitResult::Lagged(n) => {
                defmt::warn!(
                    "buddy_control: subscriber lagged, dropped {=u64} message(s)",
                    n
                );
                continue;
            }
        };
        handle(message, boot).await;
    }
}

/// React to one inbound message.
async fn handle(message: Inbound, boot: Instant) {
    match message {
        Inbound::Cmd(Cmd::Status) => reply_status(boot),
        Inbound::Cmd(Cmd::Owner { name }) => {
            defmt::info!("buddy_control: owner = {=str}", name.as_str());
            ack("owner", true, 0, None);
        }
        Inbound::Cmd(Cmd::SetName { name }) => {
            defmt::info!(
                "buddy_control: name → {=str} (persistence pending)",
                name.as_str()
            );
            ack("name", true, 0, None);
        }
        Inbound::Cmd(Cmd::Unpair) => {
            unpair().await;
        }
        Inbound::Cmd(Cmd::CharBegin { name, total }) => {
            defmt::info!(
                "buddy_control: char_begin {=str} ({=u32}B) — rejecting (no SD writer yet)",
                name.as_str(),
                total
            );
            ack(
                "char_begin",
                false,
                0,
                Some("folder push not yet supported"),
            );
        }
        Inbound::Cmd(Cmd::File { path, size }) => {
            defmt::warn!(
                "buddy_control: stray file {=str} ({=u32}B) outside a char window",
                path.as_str(),
                size
            );
            ack("file", false, 0, Some("no folder push in progress"));
        }
        Inbound::Cmd(Cmd::Chunk { data }) => {
            defmt::warn!(
                "buddy_control: stray chunk ({=usize}B) outside a file window",
                data.len()
            );
            ack("chunk", false, 0, Some("no file in progress"));
        }
        Inbound::Cmd(Cmd::FileEnd) => ack("file_end", false, 0, Some("no file in progress")),
        Inbound::Cmd(Cmd::CharEnd) => ack("char_end", false, 0, Some("no char in progress")),
        Inbound::TimeSync {
            epoch_secs,
            tz_offset_secs,
        } => {
            defmt::info!(
                "buddy_control: time-sync epoch={=i64} tz={=i32} (RTC write pending)",
                epoch_secs,
                tz_offset_secs
            );
        }
        Inbound::Turn(turn) => render_turn(&turn),
        Inbound::Snapshot(_) => {}
    }
}

/// Build a [`StatusData`] from the firmware's current snapshot and
/// signal it as an ack reply.
fn reply_status(boot: Instant) {
    let snap = snapshot::read();
    let sensors = snapshot::read_sensors();

    let battery = snap.battery.percent.map(|pct| BatteryStatus {
        pct,
        mv: snap.battery.voltage_mv.unwrap_or(0),
        ma: 0,      // current draw isn't surfaced in the snapshot
        usb: false, // usb-attach state isn't surfaced today
    });
    // Suppress the unused-binding warning when no battery field
    // is set yet — the snapshot is empty pre-AXP2101 init.
    let _ = sensors;

    let uptime_secs =
        u32::try_from(Instant::now().duration_since(boot).as_secs()).unwrap_or(u32::MAX);
    let heap_free_bytes = u32::try_from(esp_alloc::HEAP.free()).unwrap_or(u32::MAX);

    let data = StatusData {
        name: alloc::string::String::new(),
        sec: true, // bonded sessions are required at the GATT layer for our writes
        battery,
        sys: Some(SysStatus {
            uptime_secs,
            heap_free_bytes,
        }),
        stats: None, // counters land alongside operator-visible UX in a follow-up
    };
    BUDDY_OUTBOUND.signal(Outbound::StatusAck(data));
}

/// Wipe the bonds file and send the ack. Bond wipe takes effect
/// immediately on disk; the current pairing remains live until the
/// central disconnects (the desktop typically does so after a
/// `Forget`).
async fn unpair() {
    defmt::info!("buddy_control: unpair — wiping bonds");
    let empty: HVec<trouble_host::prelude::BondInformation, 8> = HVec::new();
    bonds::save_all(&empty).await;
    ack("unpair", true, 0, None);
}

/// Push the first text content block from a completed turn onto
/// the toast band. The desktop already truncates events at 4 KB on
/// the wire; we further trim the toast preview so a single long
/// reply doesn't dominate operator attention.
fn render_turn(turn: &Turn) {
    let preview = turn
        .content
        .iter()
        .find_map(text_preview)
        .unwrap_or_default();
    if preview.is_empty() {
        defmt::trace!(
            "buddy_control: turn — no text block ({=usize} content item(s))",
            turn.content.len()
        );
        return;
    }
    toast::push(ToastLevel::Warn, &preview, HalClock.now());
}

/// Extract the leading text from a content block, capped at
/// [`TOAST_PREVIEW_BYTES`]. Returns `None` for non-text blocks.
fn text_preview(block: &ContentBlock) -> Option<alloc::string::String> {
    let text = block.text.as_deref()?;
    if text.is_empty() {
        return None;
    }
    // Walk char boundaries so a multi-byte UTF-8 codepoint never
    // gets cut mid-sequence.
    let mut taken = 0;
    let mut end = 0;
    for (i, ch) in text.char_indices() {
        if taken + ch.len_utf8() > TOAST_PREVIEW_BYTES {
            break;
        }
        end = i + ch.len_utf8();
        taken += ch.len_utf8();
    }
    Some(text[..end].to_string())
}

/// Helper that signals a generic [`Ack`] back on
/// [`BUDDY_OUTBOUND`].
fn ack(cmd: &str, ok: bool, n: u32, error: Option<&str>) {
    BUDDY_OUTBOUND.signal(Outbound::Ack(Ack {
        cmd: cmd.into(),
        ok,
        n,
        error: error.map(alloc::string::String::from),
    }));
}
