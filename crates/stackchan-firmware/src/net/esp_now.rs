//! ESP-NOW RX task — drives [`REMOTE_COMMAND_SIGNAL`] from inbound
//! ESP-NOW frames.
//!
//! ## Responsibilities
//!
//! 1. Configure the radio from `STACKCHAN.RON`'s `esp_now` block:
//!    install the PMK, register a static peer if `peer_mac` is set,
//!    optionally lock the channel.
//! 2. Loop on `EspNow::receive_async`, drop anything that doesn't
//!    parse as a valid Stack-chan frame, route the rest into the
//!    same `RemoteCommand` plumbing the HTTP control plane uses.
//! 3. Honour the pairing window — when [`PAIR_WINDOW`] is signalled
//!    (by `POST /pair`), accept frames from senders that aren't on
//!    the static-peer allowlist; outside the window, drop them.
//!
//! ## Why not split RX/TX yet
//!
//! TX (the 5 Hz heartbeat) is the natural follow-up; this PR ships
//! the RX path so an external `M5StickC` remote can drive the avatar.
//! The split keeps the diff focused — TX is its own task, its own
//! lifecycle, its own failure modes.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant};
use esp_radio::esp_now::{BROADCAST_ADDRESS, EspNow, EspNowWifiInterface, PeerInfo, ReceivedData};
use stackchan_net::EspNowConfig;
use stackchan_net::config::parse_mac;
use stackchan_net::esp_now::{InboundFrame, decode};

use crate::net::http::REMOTE_COMMAND_SIGNAL;

/// Pairing-window deadline.
///
/// Producers (HTTP `POST /pair`, MCP `enter_pairing` tool) signal this
/// with `now + duration` when an operator opens a pairing window. The
/// RX task gates non-allowlisted frames on the deadline being in the
/// future. `None` (Signal not yet signalled) is the closed default.
pub static PAIR_WINDOW: Signal<CriticalSectionRawMutex, Instant> = Signal::new();

/// Latched ON when the ESP-NOW task is running. Read by the dashboard
/// via the `/state` snapshot for operator-visible debug.
pub static ESP_NOW_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Cached pairing deadline. The Signal is single-waker; the task
/// drains it into this static so multiple frame-handling paths can
/// peek at the deadline without contending for the waker slot.
struct PairWindowCache {
    /// `None` = window closed; `Some(t)` = open until `t`.
    until: Option<Instant>,
}

impl PairWindowCache {
    /// Construct with the window closed.
    const fn new() -> Self {
        Self { until: None }
    }

    /// Refresh from the Signal if a new deadline arrived; report
    /// whether the window is currently open.
    fn refresh_and_check(&mut self, now: Instant) -> bool {
        if let Some(new_until) = PAIR_WINDOW.try_take() {
            self.until = Some(new_until);
        }
        match self.until {
            Some(until) if now < until => true,
            Some(_) => {
                self.until = None;
                false
            }
            None => false,
        }
    }
}

/// ESP-NOW receive task.
///
/// Boots inert if `cfg.enabled == false` — the task exits without
/// touching the radio. Otherwise installs the PMK, registers the
/// static peer (if configured), optionally locks the channel, then
/// loops on `receive`.
#[embassy_executor::task]
pub async fn esp_now_task(esp_now: EspNow<'static>, cfg: EspNowConfig) {
    if !cfg.enabled {
        defmt::info!("esp-now: disabled in config — task exiting");
        return;
    }
    let (manager, _sender, mut receiver) = esp_now.split();

    // Install PMK if configured. Empty string = no encryption (dev mode).
    if !cfg.pmk_hex.is_empty() {
        if let Some(pmk) = parse_hex_key(&cfg.pmk_hex) {
            if let Err(e) = manager.set_pmk(&pmk) {
                defmt::warn!("esp-now: set_pmk failed: {}", defmt::Debug2Format(&e));
            }
        } else {
            defmt::warn!(
                "esp-now: pmk_hex didn't parse — skipping (config validator should have caught this)"
            );
        }
    }

    // Register the static peer if configured. Failure here is
    // recoverable — we still want to accept dynamic peers via the
    // pairing window.
    if let Some(peer_mac) = parse_mac(&cfg.peer_mac) {
        let lmk = if cfg.lmk_hex.is_empty() {
            None
        } else {
            parse_hex_key(&cfg.lmk_hex)
        };
        let encrypt = !cfg.pmk_hex.is_empty() && lmk.is_some();
        let peer = PeerInfo {
            interface: EspNowWifiInterface::Sta,
            peer_address: peer_mac,
            lmk,
            channel: cfg.channel,
            encrypt,
        };
        match manager.add_peer(peer) {
            Ok(()) => defmt::info!(
                "esp-now: static peer registered ({=[u8; 6]:02x}) encrypt={}",
                peer_mac,
                encrypt
            ),
            Err(e) => defmt::warn!("esp-now: add_peer failed: {}", defmt::Debug2Format(&e)),
        }
    }

    // Lock the radio channel if the operator pinned one. With
    // `channel = None` the radio follows whatever the Wi-Fi STA
    // associated on, which is the standard mode.
    if let Some(ch) = cfg.channel
        && let Err(e) = manager.set_channel(ch)
    {
        defmt::warn!(
            "esp-now: set_channel({}) failed: {}",
            ch,
            defmt::Debug2Format(&e)
        );
    }

    ESP_NOW_ACTIVE.store(true, Ordering::Release);
    defmt::info!(
        "esp-now: RX task ready (peer={=str:?})",
        cfg.peer_mac.as_str()
    );

    let static_peer = parse_mac(&cfg.peer_mac);
    let mut window = PairWindowCache::new();

    loop {
        let frame = receiver.receive_async().await;
        let now = Instant::now();
        let allow = is_allowlisted(&frame, static_peer, &mut window, now);
        if !allow {
            // Drop silently — emitting a log per dropped frame would
            // flood the bus on a noisy 2.4 GHz environment.
            continue;
        }
        match decode(frame.data()) {
            Ok(InboundFrame::Heartbeat) => {
                // Liveness only; no state change.
            }
            Ok(InboundFrame::Command(cmd)) => {
                REMOTE_COMMAND_SIGNAL.signal(cmd);
            }
            Err(e) => {
                defmt::warn!(
                    "esp-now: drop frame ({=usize} bytes): {}",
                    frame.data().len(),
                    defmt::Debug2Format(&e)
                );
            }
        }
    }
}

/// Allowlist check: accept frames from the configured static peer
/// always; accept anything during a pairing window; drop the rest.
/// Broadcast-destination frames pass — the firmware is the intended
/// audience for the broadcast pattern that bulk operator tooling uses.
fn is_allowlisted(
    frame: &ReceivedData,
    static_peer: Option<[u8; 6]>,
    window: &mut PairWindowCache,
    now: Instant,
) -> bool {
    let src = frame.info.src_address;
    if let Some(allowed) = static_peer
        && src == allowed
    {
        return true;
    }
    if frame.info.dst_address == BROADCAST_ADDRESS && static_peer == Some(src) {
        return true;
    }
    window.refresh_and_check(now)
}

/// Helper: parse a 32-character hex key into 16 bytes. Validator
/// already vetted shape; returns `None` only on internal mismatch
/// (which would be a bug).
fn parse_hex_key(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    let bytes = s.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}

/// ASCII hex nibble decoder mirroring the validator helper in
/// `stackchan-net::config` — duplicated here because that fn is
/// `pub(crate)` and the binary boundary keeps it that way.
const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Default pairing window when the HTTP route is hit without explicit
/// duration. Mirrors `stackchan_net::http_command::DEFAULT_PAIRING_DURATION_MS`.
pub const PAIR_WINDOW_DEFAULT_MS: u64 = 30_000;

/// Open the pairing window for `duration_ms`. Producer-side helper
/// for the HTTP route + MCP tool: signals [`PAIR_WINDOW`] with the
/// computed deadline.
pub fn open_pair_window(duration_ms: u32) {
    let until = Instant::now() + Duration::from_millis(u64::from(duration_ms));
    PAIR_WINDOW.signal(until);
}
