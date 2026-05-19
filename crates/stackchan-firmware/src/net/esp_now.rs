//! ESP-NOW task — RX (drives [`REMOTE_COMMAND_SIGNAL`] from inbound
//! frames) plus TX (broadcasts pose-mirror + heartbeat frames so other
//! Stack-chan units can choreograph against this device).
//!
//! ## Responsibilities
//!
//! 1. Configure the radio from `STACKCHAN.RON`'s `esp_now` block:
//!    install the PMK, register a static peer if `peer_mac` is set,
//!    optionally lock the channel.
//! 2. RX: receive frames, drop anything that doesn't parse as a valid
//!    Stack-chan frame, route commands into the same
//!    `RemoteCommand` plumbing the HTTP control plane uses.
//! 3. RX: honour the pairing window — when [`PAIR_WINDOW`] is signalled
//!    (by `POST /pair`), accept frames from senders that aren't on
//!    the static-peer allowlist; outside the window, drop them.
//! 4. TX: every `TX_TICK_MS` poll the avatar snapshot; emit a
//!    `PoseMirror` frame when the pose moved by more than
//!    `POSE_TX_EPSILON_DEG`, and a `Heartbeat` frame at
//!    `HEARTBEAT_INTERVAL_MS` cadence (skipped when a pose-mirror
//!    went out in the same tick — they double as liveness).
//!
//! TX targets the broadcast MAC. ESP-NOW broadcast can't be encrypted,
//! which matches the choreography use case: any unit on the same
//! channel can listen without pre-shared keys. The asymmetric
//! configured-peer encryption stays on the RX side.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use esp_radio::esp_now::{BROADCAST_ADDRESS, EspNow, EspNowWifiInterface, PeerInfo, ReceivedData};
use stackchan_core::Pose;
use stackchan_net::EspNowConfig;
use stackchan_net::config::parse_mac;
use stackchan_net::esp_now::{
    InboundFrame, MAX_FRAME_LEN, decode, encode_heartbeat, encode_pose_mirror,
};

use crate::net::http::REMOTE_COMMAND_SIGNAL;
use crate::net::snapshot::read as read_avatar_snapshot;

/// TX cadence — how often the loop checks the snapshot for a new
/// pose. 200 ms (5 Hz) keeps mirroring smooth without flooding the
/// 2.4 GHz band with sub-perceptible deltas.
const TX_TICK_MS: u64 = 200;

/// Pose change threshold for emitting a `PoseMirror`. Below this the
/// pose is treated as held — quantising avoids broadcasting servo
/// jitter or sub-degree noise that wouldn't be perceptible on a
/// receiver's avatar.
const POSE_TX_EPSILON_DEG: f32 = 0.5;

/// How often a `Heartbeat` frame is sent independent of pose changes.
/// 1 Hz is plenty of liveness signal — receivers that need faster
/// disconnect detection can subscribe to other Stack-chan TX traffic.
const HEARTBEAT_INTERVAL_MS: u64 = 1_000;

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
    let (manager, mut sender, mut receiver) = esp_now.split();

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

    // Register the broadcast peer so `sender.send_async` to
    // BROADCAST_ADDRESS doesn't return PeerNotFound. ESP-NOW broadcast
    // is unencrypted by spec — no LMK, encrypt=false.
    let broadcast_peer = PeerInfo {
        interface: EspNowWifiInterface::Sta,
        peer_address: BROADCAST_ADDRESS,
        lmk: None,
        channel: cfg.channel,
        encrypt: false,
    };
    if let Err(e) = manager.add_peer(broadcast_peer) {
        // PeerExists is fine — esp-radio may auto-add the broadcast
        // peer on first use. Other errors block TX but not RX.
        defmt::info!("esp-now: broadcast peer add: {}", defmt::Debug2Format(&e));
    }

    ESP_NOW_ACTIVE.store(true, Ordering::Release);
    defmt::info!(
        "esp-now: task ready (peer={=str:?}, tx tick {=u64} ms, heartbeat {=u64} ms)",
        cfg.peer_mac.as_str(),
        TX_TICK_MS,
        HEARTBEAT_INTERVAL_MS,
    );

    let static_peer = parse_mac(&cfg.peer_mac);
    let mut window = PairWindowCache::new();
    let mut tx_state = TxState::new();
    let mut tx_deadline = Instant::now() + Duration::from_millis(TX_TICK_MS);

    loop {
        // The Timer here is just a wake-up — the actual TX trigger
        // is `Instant::now() >= tx_deadline` after the select
        // resolves. Earlier versions used the `Either::Second` arm
        // as the TX trigger, but `select` polls its first future
        // first, so under sustained inbound traffic `Either::First`
        // fired every iteration, dropping the timer mid-flight and
        // never advancing past it. The wall-clock deadline persists
        // across cancelled-future cycles, so RX traffic can't starve
        // TX. (See greptile review on PR #255.)
        let timeout = tx_deadline.saturating_duration_since(Instant::now());
        match select(receiver.receive_async(), Timer::after(timeout)).await {
            Either::First(frame) => {
                handle_inbound_frame(&frame, static_peer, &mut window);
            }
            Either::Second(()) => {
                // Timer fired naturally; deadline is now in the past.
            }
        }
        if Instant::now() >= tx_deadline {
            tx_state.tick(&mut sender).await;
            tx_deadline = Instant::now() + Duration::from_millis(TX_TICK_MS);
        }
    }
}

/// Per-frame inbound handling — allowlist gate + decode + dispatch.
/// Pulled out so the receive arm of the `select!` stays a single
/// statement and the loop body reads top-down.
fn handle_inbound_frame(
    frame: &ReceivedData,
    static_peer: Option<[u8; 6]>,
    window: &mut PairWindowCache,
) {
    let now = Instant::now();
    if !is_allowlisted(frame, static_peer, window, now) {
        // Drop silently — emitting a log per dropped frame would
        // flood the bus on a noisy 2.4 GHz environment.
        return;
    }
    match decode(frame.data()) {
        // Heartbeat = liveness only. Pose-mirror frames from peer
        // Stack-chans are advisory only — this firmware doesn't
        // slave its head to peer poses (would invite multi-unit
        // feedback loops). A future choreography modifier can
        // subscribe via a separate signal if that policy changes.
        Ok(InboundFrame::Heartbeat | InboundFrame::Pose { .. }) => {}
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

/// TX-side state machine: tracks the last broadcast pose so we only
/// emit `PoseMirror` when the avatar's head has actually moved, plus
/// a heartbeat deadline so liveness frames go out at a steady cadence
/// regardless of pose activity.
struct TxState {
    /// Last pose actually emitted on the wire. `None` until the
    /// first send so the very first non-zero pose is always
    /// announced.
    last_sent: Option<Pose>,
    /// Wall-clock time at which the next heartbeat is due. Reset
    /// after every successful heartbeat send.
    next_heartbeat_at: Instant,
}

impl TxState {
    /// Construct with no prior pose recorded and the first heartbeat
    /// scheduled one full interval out — the very first tick should
    /// announce the boot pose without doubling up with a heartbeat.
    fn new() -> Self {
        Self {
            last_sent: None,
            next_heartbeat_at: Instant::now() + Duration::from_millis(HEARTBEAT_INTERVAL_MS),
        }
    }

    /// Emit pose-mirror + heartbeat as appropriate. The heartbeat is
    /// suppressed for the current tick if a pose-mirror went out
    /// (the receiver gets liveness for free from the pose frame).
    async fn tick(&mut self, sender: &mut esp_radio::esp_now::EspNowSender<'_>) {
        let snapshot = read_avatar_snapshot();
        // Coerce non-finite axes to zero before comparison or storage.
        // The encoder already maps NaN/inf to 0 on the wire, so a
        // raw NaN snapshot would still send `(0, 0)`; without the
        // pre-clamp here, `last_sent` would latch the NaN and every
        // subsequent `pose_delta_exceeds(NaN, valid, ε)` would
        // evaluate `false`, killing TX until reboot. (See greptile
        // review on PR #255.)
        let pose = Pose::new(
            if snapshot.head_pose.pan_deg.is_finite() {
                snapshot.head_pose.pan_deg
            } else {
                0.0
            },
            if snapshot.head_pose.tilt_deg.is_finite() {
                snapshot.head_pose.tilt_deg
            } else {
                0.0
            },
        );
        let pose_changed = self
            .last_sent
            .is_none_or(|prev| pose_delta_exceeds(prev, pose, POSE_TX_EPSILON_DEG));
        let now = Instant::now();

        let sent_anything = if pose_changed && send_pose_mirror(sender, pose).await {
            self.last_sent = Some(pose);
            true
        } else {
            false
        };

        // Pose-mirror counts as liveness; heartbeat fires only when
        // the deadline passed and no pose-mirror went out this tick.
        // Either path advances the heartbeat deadline by one full
        // interval so we don't double up.
        let heartbeat_sent =
            !sent_anything && now >= self.next_heartbeat_at && send_heartbeat(sender).await;
        if sent_anything || heartbeat_sent {
            self.next_heartbeat_at = now + Duration::from_millis(HEARTBEAT_INTERVAL_MS);
        }
    }
}

/// True iff the angular delta between `a` and `b` exceeds
/// `epsilon_deg` on either axis.
fn pose_delta_exceeds(a: Pose, b: Pose, epsilon_deg: f32) -> bool {
    (a.pan_deg - b.pan_deg).abs() > epsilon_deg || (a.tilt_deg - b.tilt_deg).abs() > epsilon_deg
}

/// Encode + transmit a `PoseMirror` frame. Returns `true` on a
/// successful send; logs and returns `false` on encode / send errors
/// so the caller doesn't update `last_sent` on a dropped frame.
async fn send_pose_mirror(sender: &mut esp_radio::esp_now::EspNowSender<'_>, pose: Pose) -> bool {
    let mut buf = [0_u8; MAX_FRAME_LEN];
    let len = match encode_pose_mirror(&mut buf, pose.pan_deg, pose.tilt_deg) {
        Ok(n) => n,
        Err(e) => {
            defmt::warn!(
                "esp-now tx: encode pose-mirror failed: {}",
                defmt::Debug2Format(&e)
            );
            return false;
        }
    };
    if let Err(e) = sender.send_async(&BROADCAST_ADDRESS, &buf[..len]).await {
        defmt::warn!(
            "esp-now tx: pose-mirror send failed: {}",
            defmt::Debug2Format(&e)
        );
        return false;
    }
    true
}

/// Encode + transmit a `Heartbeat` frame. Same error-swallowing
/// shape as [`send_pose_mirror`].
async fn send_heartbeat(sender: &mut esp_radio::esp_now::EspNowSender<'_>) -> bool {
    let mut buf = [0_u8; 8];
    let len = match encode_heartbeat(&mut buf) {
        Ok(n) => n,
        Err(e) => {
            defmt::warn!(
                "esp-now tx: encode heartbeat failed: {}",
                defmt::Debug2Format(&e)
            );
            return false;
        }
    };
    if let Err(e) = sender.send_async(&BROADCAST_ADDRESS, &buf[..len]).await {
        defmt::warn!(
            "esp-now tx: heartbeat send failed: {}",
            defmt::Debug2Format(&e)
        );
        return false;
    }
    true
}

/// Allowlist check: accept frames from the configured static peer
/// always; accept anything during a pairing window; drop the rest.
///
/// Broadcasts from random senders fall through to the pairing-window
/// gate intentionally — accepting unicast operator commands from the
/// LAN at large would be an open RX port, so the broadcast pattern
/// only helps once an operator has explicitly opened pairing.
fn is_allowlisted(
    frame: &ReceivedData,
    static_peer: Option<[u8; 6]>,
    window: &mut PairWindowCache,
    now: Instant,
) -> bool {
    let src = frame.info.src_address;
    if static_peer == Some(src) {
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

/// Open the pairing window for `duration_ms`. Producer-side helper
/// for the HTTP route + MCP tool: signals [`PAIR_WINDOW`] with the
/// computed deadline.
pub fn open_pair_window(duration_ms: u32) {
    let until = Instant::now() + Duration::from_millis(u64::from(duration_ms));
    PAIR_WINDOW.signal(until);
}
