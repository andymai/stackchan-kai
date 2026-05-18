//! Desktop command-surface handler: status / owner / name / unpair
//! / time-sync / turn events / folder push.
//!
//! Subscribes to [`crate::ble::desktop::DESKTOP_INBOUND`] for the
//! `cmd`- and `evt`-tagged messages the other desktop tasks
//! ([`crate::desktop_render`], [`crate::desktop_permission`]) don't
//! consume themselves, and signals replies on
//! [`crate::ble::desktop::DESKTOP_OUTBOUND`].
//!
//! - **status** → builds a [`StatusData`] from the current
//!   battery / uptime / heap / approval+deny counters and replies.
//! - **owner** → logs + acks.
//! - **name** → writes `/sd/DEVICE.NAM` and soft-resets so the
//!   new name takes effect on the next advertise cycle (the BLE
//!   local name is captured into a `StaticCell` early in `main`).
//!   The ack flushes before the reset.
//! - **unpair** → wipes the SD-backed bonds via
//!   [`crate::ble::bonds::save_all`] with an empty list, then acks.
//! - **time** (`{"time":[epoch,tz]}`) → fans the epoch out to
//!   [`crate::desktop_time::DESKTOP_RTC_WRITE_REQUEST`] for the
//!   dedicated writer task; tz is logged only (BM8563 stores UTC).
//! - **turn** events → push the assistant's first text block into
//!   the toast band so the operator sees the recent reply.
//! - **`char_begin` / `file` / `chunk` / `file_end` / `char_end`** →
//!   buffer each pushed file in PSRAM and commit on `file_end` via
//!   [`crate::storage::FirmwareStorage::write_desktop_file`]. Files
//!   land at `/sd/desktop/<char>/<path>`. Path components are
//!   validated via
//!   [`stackchan_desktop_protocol::is_safe_relative_path`] before
//!   any disk touch; mismatched / out-of-sequence opcodes get an
//!   `ok:false` ack so the desktop's drop target surfaces a clear
//!   failure.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use embassy_sync::pubsub::WaitResult;
use embassy_time::{Duration, Instant};
use heapless_09::Vec as HVec;
use stackchan_core::Clock;
use stackchan_desktop_protocol::{
    Ack, BatteryStatus, Cmd, ContentBlock, Inbound, Outbound, StatusData, SysStatus, Turn,
    is_safe_relative_path,
};

use crate::ble::bonds;
use crate::ble::desktop::{DESKTOP_INBOUND, DESKTOP_OUTBOUND};
use crate::clock::HalClock;
use crate::net::snapshot;
use crate::storage::{self, FirmwareStorage};
use crate::toast::{self, ToastLevel};

/// Cap on how much of an assistant text block we push to the toast
/// band. The toast itself truncates by codepoints, but capping
/// before push avoids re-walking long assistant essays on every
/// turn event.
const TOAST_PREVIEW_BYTES: usize = 64;

/// Per-file byte cap for the folder-push buffer. Sized to absorb
/// the largest character asset the operator is realistically going
/// to drop — a long-tail GIF or a few seconds of mono PCM. The
/// total-payload spec cap is `FOLDER_PUSH_MAX_BYTES` (~1.8 MB) but
/// that's the sum across all files in a char; any one file
/// larger than this gets rejected with `ok:false` to keep PSRAM
/// pressure bounded.
const MAX_DESKTOP_FILE_BYTES: u32 = 262_144;

/// Command-surface task. Spawns once at boot, runs for the
/// firmware lifetime.
#[embassy_executor::task]
pub async fn desktop_control_task() -> ! {
    let Ok(mut sub) = DESKTOP_INBOUND.subscriber() else {
        defmt::error!(
            "desktop_control: DESKTOP_INBOUND subscriber slot exhausted; task parking forever"
        );
        loop {
            embassy_time::Timer::after(Duration::from_secs(3600)).await;
        }
    };
    let boot = Instant::now();
    let mut push = PushState::default();
    defmt::info!(
        "desktop_control: armed (status / owner / name / unpair / turn / push → /sd/desktop/)"
    );

    loop {
        let message = match sub.next_message().await {
            WaitResult::Message(m) => m,
            WaitResult::Lagged(n) => {
                defmt::warn!(
                    "desktop_control: subscriber lagged, dropped {=u64} message(s)",
                    n
                );
                continue;
            }
        };
        handle(message, boot, &mut push).await;
    }
}

/// Folder-push pipeline state. Drops back to default on any
/// failure / completion / out-of-sequence opcode so the receiver
/// can't get wedged between drag attempts.
#[derive(Debug, Default)]
struct PushState {
    /// `<char_name>` from the most recent successful `char_begin`.
    /// Path-validated; `None` means "no folder push in flight."
    active_char: Option<String>,
    /// File metadata + buffer for the file currently streaming. The
    /// `Vec` collects chunk bytes until `file_end` flushes them to
    /// SD via [`FirmwareStorage::write_desktop_file`].
    current_file: Option<CurrentFile>,
}

/// One in-flight file inside a folder push.
#[derive(Debug)]
struct CurrentFile {
    /// `<file_name>` from the `file` opcode. Path-validated.
    name: String,
    /// Bytes accumulated from `chunk` opcodes; flushed + reset on
    /// `file_end`.
    buffer: Vec<u8>,
}

/// React to one inbound message.
async fn handle(message: Inbound, boot: Instant, push: &mut PushState) {
    match message {
        Inbound::Cmd(Cmd::Status) => reply_status(boot),
        Inbound::Cmd(Cmd::Owner { name }) => {
            defmt::info!("desktop_control: owner = {=str}", name.as_str());
            ack("owner", true, 0, None);
        }
        Inbound::Cmd(Cmd::SetName { name }) => {
            set_name(&name).await;
        }
        Inbound::Cmd(Cmd::Unpair) => {
            unpair().await;
        }
        Inbound::Cmd(Cmd::CharBegin { name, total }) => {
            char_begin(push, &name, total);
        }
        Inbound::Cmd(Cmd::File { path, size }) => {
            file_announce(push, &path, size);
        }
        Inbound::Cmd(Cmd::Chunk { data }) => {
            chunk(push, &data);
        }
        Inbound::Cmd(Cmd::FileEnd) => file_end(push).await,
        Inbound::Cmd(Cmd::CharEnd) => char_end(push),
        Inbound::TimeSync {
            epoch_secs,
            tz_offset_secs,
        } => {
            defmt::info!(
                "desktop_control: time-sync epoch={=i64} tz={=i32}",
                epoch_secs,
                tz_offset_secs
            );
            // Hand the epoch off to the dedicated writer task; tz
            // is logged-only today (the BM8563 stores UTC).
            crate::desktop_time::DESKTOP_RTC_WRITE_REQUEST.signal(epoch_secs);
        }
        Inbound::Turn(turn) => render_turn(&turn),
        Inbound::Snapshot(_) => {}
    }
}

/// Start a new folder push. Reject path-unsafe `<name>`; drop any
/// stale state from a previously-interrupted push so the device
/// can't get wedged in a half-state across drag attempts.
fn char_begin(push: &mut PushState, name: &str, total: u32) {
    if !is_safe_relative_path(name) {
        defmt::warn!("desktop_control: char_begin {=str} — unsafe path", name);
        ack("char_begin", false, 0, Some("unsafe char name"));
        return;
    }
    // Reset any stragglers from a prior interrupted push.
    *push = PushState::default();
    push.active_char = Some(name.to_string());
    defmt::info!(
        "desktop_control: char_begin {=str} ({=u32}B total)",
        name,
        total
    );
    ack("char_begin", true, 0, None);
}

/// Announce a single file inside the current char. Caps the
/// per-file size and validates the path before allocating the
/// chunk buffer.
fn file_announce(push: &mut PushState, path: &str, size: u32) {
    if push.active_char.is_none() {
        defmt::warn!(
            "desktop_control: stray file {=str} outside a char window",
            path
        );
        ack("file", false, 0, Some("no folder push in progress"));
        return;
    }
    if !is_safe_relative_path(path) {
        defmt::warn!("desktop_control: file {=str} — unsafe path", path);
        ack("file", false, 0, Some("unsafe file path"));
        return;
    }
    if size > MAX_DESKTOP_FILE_BYTES {
        defmt::warn!(
            "desktop_control: file {=str} too large ({=u32}B > {=u32}B cap)",
            path,
            size,
            MAX_DESKTOP_FILE_BYTES
        );
        ack("file", false, 0, Some("file exceeds size cap"));
        return;
    }
    push.current_file = Some(CurrentFile {
        name: path.to_string(),
        buffer: Vec::with_capacity(size as usize),
    });
    ack("file", true, 0, None);
}

/// Append one chunk's bytes to the in-flight file buffer. The `n`
/// counter in the ack reports total bytes accumulated for this
/// file so far — the desktop uses it as a sanity check against
/// its own send progress.
fn chunk(push: &mut PushState, data: &[u8]) {
    let Some(file) = push.current_file.as_mut() else {
        defmt::warn!(
            "desktop_control: stray chunk ({=usize}B) outside a file window",
            data.len()
        );
        ack("chunk", false, 0, Some("no file in progress"));
        return;
    };
    // Defensive: per-file cap is enforced at `file` announce, but
    // a misbehaving desktop could send more bytes than the
    // announced `size`. Reject the overflow rather than growing
    // the Vec unbounded.
    if file.buffer.len().saturating_add(data.len()) > MAX_DESKTOP_FILE_BYTES as usize {
        defmt::warn!(
            "desktop_control: chunk would exceed cap ({=usize}+{=usize}B > {=u32}B)",
            file.buffer.len(),
            data.len(),
            MAX_DESKTOP_FILE_BYTES
        );
        ack("chunk", false, 0, Some("file exceeds size cap"));
        // Drop the half-written file so a subsequent `file_end`
        // doesn't commit truncated bytes to disk.
        push.current_file = None;
        return;
    }
    file.buffer.extend_from_slice(data);
    let n = u32::try_from(file.buffer.len()).unwrap_or(u32::MAX);
    ack("chunk", true, n, None);
}

/// Flush the in-flight file to SD via
/// [`FirmwareStorage::write_desktop_file`]. On any SD error the
/// ack reports `ok:false`; the buffer is dropped either way so a
/// subsequent `file` opcode starts clean.
async fn file_end(push: &mut PushState) {
    let Some(file) = push.current_file.take() else {
        ack("file_end", false, 0, Some("no file in progress"));
        return;
    };
    let Some(char_name) = push.active_char.clone() else {
        // Shouldn't happen — file_announce guards on active_char —
        // but be defensive.
        ack("file_end", false, 0, Some("no folder push in progress"));
        return;
    };
    let final_size = u32::try_from(file.buffer.len()).unwrap_or(u32::MAX);
    let outcome = storage::with_storage(|s: &mut FirmwareStorage| {
        s.write_desktop_file(&char_name, &file.name, &file.buffer)
    })
    .await;
    match outcome {
        Some(Ok(())) => {
            defmt::info!(
                "desktop_control: wrote /sd/desktop/{=str}/{=str} ({=u32}B)",
                char_name.as_str(),
                file.name.as_str(),
                final_size,
            );
            ack("file_end", true, final_size, None);
        }
        Some(Err(e)) => {
            defmt::warn!(
                "desktop_control: write_desktop_file failed ({})",
                defmt::Debug2Format(&e)
            );
            ack("file_end", false, 0, Some("sd write failed"));
        }
        None => {
            defmt::warn!("desktop_control: no SD mounted; dropping file");
            ack("file_end", false, 0, Some("no sd mounted"));
        }
    }
}

/// End the current folder push. Always succeeds — the per-file
/// commits happened in `file_end`; `char_end` is just the
/// teardown signal.
fn char_end(push: &mut PushState) {
    if let Some(name) = push.active_char.take() {
        defmt::info!("desktop_control: char_end {=str}", name.as_str());
        ack("char_end", true, 0, None);
    } else {
        ack("char_end", false, 0, Some("no char in progress"));
    }
    // `current_file` should already be None after `file_end`; drop
    // any stray buffer defensively.
    push.current_file = None;
}

/// Build a [`StatusData`] from the firmware's current snapshot and
/// signal it as an ack reply.
fn reply_status(boot: Instant) {
    let snap = snapshot::read();

    let battery = snap.battery.percent.map(|pct| BatteryStatus {
        pct,
        mv: snap.battery.voltage_mv.unwrap_or(0),
        ma: 0,      // current draw isn't surfaced in the snapshot
        usb: false, // usb-attach state isn't surfaced today
    });

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
    DESKTOP_OUTBOUND.signal(Outbound::StatusAck(data));
}

/// Persist the desktop-supplied BLE name to `/sd/DEVICE.NAM` and
/// soft-reset the device so the new name takes effect on the next
/// advertise cycle.
///
/// The BLE local name is captured into a `StaticCell` early in
/// `main`; there's no in-flight way to swap it. The reboot avoids
/// the more invasive "hot-swap the advertise name" refactor that
/// would otherwise be needed. Operators see the reboot as the
/// device cycling once after they hit "Save" in the desktop's
/// Hardware Buddy panel — a one-second blip, then the device
/// reappears with the new name.
///
/// On SD-absent / write-failure paths the ack reports `ok: false`
/// with the reason and we skip the reset (no point cycling if the
/// new name isn't actually on disk).
async fn set_name(name: &str) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        defmt::warn!("desktop_control: name → empty; rejecting without reboot");
        ack("name", false, 0, Some("empty name"));
        return;
    }
    defmt::info!("desktop_control: name → {=str}", trimmed);
    let outcome = crate::storage::with_storage(|s| s.write_device_name(trimmed)).await;
    match outcome {
        Some(Ok(())) => {
            ack("name", true, 0, None);
            // Best-effort: give the BLE notify path a tick to flush
            // the ack onto the wire before we reboot, otherwise the
            // desktop sees the disconnect first and may interpret
            // the missing ack as failure.
            embassy_time::Timer::after(Duration::from_millis(250)).await;
            defmt::info!("desktop_control: rebooting to pick up new BLE name");
            esp_hal::system::software_reset();
        }
        Some(Err(crate::storage::StorageError::TooLarge)) => {
            defmt::warn!("desktop_control: name too long (> 22 bytes)");
            ack("name", false, 0, Some("name too long"));
        }
        Some(Err(e)) => {
            defmt::warn!(
                "desktop_control: write_device_name failed ({})",
                defmt::Debug2Format(&e)
            );
            ack("name", false, 0, Some("sd write failed"));
        }
        None => {
            defmt::warn!("desktop_control: no SD mounted; cannot persist name");
            ack("name", false, 0, Some("no sd mounted"));
        }
    }
}

/// Wipe the bonds file and send the ack. Bond wipe takes effect
/// immediately on disk; the current pairing remains live until the
/// central disconnects (the desktop typically does so after a
/// `Forget`).
///
/// The ack reports the wipe's actual outcome: if the SD card is
/// absent or the write fails, `ok: false` goes back to the desktop
/// (otherwise on next boot the unchanged bonds would silently
/// re-pair). Reasons are short fixed strings so defmt + the wire
/// stay terse.
async fn unpair() {
    defmt::info!("desktop_control: unpair — wiping bonds");
    let empty: HVec<trouble_host::prelude::BondInformation, 8> = HVec::new();
    match bonds::save_all(&empty).await {
        Ok(()) => ack("unpair", true, 0, None),
        Err(reason) => ack("unpair", false, 0, Some(reason)),
    }
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
            "desktop_control: turn — no text block ({=usize} content item(s))",
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
    // gets cut mid-sequence. `char_indices` yields byte offsets;
    // the next-char check is `i + ch.len_utf8() > cap` so the
    // longest prefix that fits remains in `end`.
    let mut end = 0;
    for (i, ch) in text.char_indices() {
        if i + ch.len_utf8() > TOAST_PREVIEW_BYTES {
            break;
        }
        end = i + ch.len_utf8();
    }
    Some(text[..end].to_string())
}

/// Helper that signals a generic [`Ack`] back on
/// [`DESKTOP_OUTBOUND`].
fn ack(cmd: &str, ok: bool, n: u32, error: Option<&str>) {
    DESKTOP_OUTBOUND.signal(Outbound::Ack(Ack {
        cmd: cmd.into(),
        ok,
        n,
        error: error.map(alloc::string::String::from),
    }));
}
