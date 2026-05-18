//! Render snapshots from Claude Desktop onto the avatar's existing
//! affect surfaces.
//!
//! Subscribes to [`crate::ble::desktop::DESKTOP_INBOUND`] and translates
//! each [`Inbound`] into something the firmware's existing channels
//! already understand:
//!
//! - `Snapshot { waiting > 0, .. }` → curious emotion + toast band
//!   carrying the desktop's one-line `msg`
//! - `Snapshot { running > 0, .. }` → happy emotion (engaged, no
//!   blocking prompt)
//! - `Snapshot { total == 0 }` → release autonomy (`Reset`) so the
//!   avatar idles
//! - `Cmd::Owner { name }` → log only for now (persistence joins the
//!   owner / status / unpair handlers in a later slice)
//! - everything else → log only
//!
//! Emotion holds are re-asserted on every snapshot the desktop
//! sends. The reference protocol guarantees a keepalive every 10 s,
//! so a 25 s hold safely covers two consecutive keepalive intervals
//! without letting autonomy slip in between, and a dropped link
//! falls back to autonomy within one hold window.

use embassy_sync::pubsub::WaitResult;
use stackchan_core::{Clock, Emotion, RemoteCommand};
use stackchan_desktop_protocol::{Cmd, Inbound, Snapshot};

use crate::ble::desktop::DESKTOP_INBOUND;
use crate::clock::HalClock;
use crate::net::http::REMOTE_COMMAND_SIGNAL;
use crate::toast::{self, ToastLevel};

/// Hold each derived emotion for two heartbeat keepalive intervals.
/// The desktop guarantees a 10 s keepalive cadence; 25 s covers two
/// missed keepalives before autonomy resumes — long enough to ride
/// out one dropped notification but short enough to fall back if the
/// link silently dies.
const EMOTION_HOLD_MS: u32 = 25_000;

/// Render task. Spawns once at boot, runs for the firmware lifetime.
/// Parks until the first inbound, then loops one message at a time.
/// Lost messages from a back-pressured subscriber slot surface as a
/// warning — at ≤10 Hz the queue should never genuinely back up.
#[embassy_executor::task]
pub async fn desktop_render_task() -> ! {
    let Ok(mut sub) = DESKTOP_INBOUND.subscriber() else {
        defmt::error!(
            "desktop_render: DESKTOP_INBOUND subscriber slot exhausted; task parking forever"
        );
        loop {
            embassy_time::Timer::after(embassy_time::Duration::from_secs(3600)).await;
        }
    };
    defmt::info!("desktop_render: subscribed to DESKTOP_INBOUND");

    let mut last_emotion: Option<Emotion> = None;

    loop {
        let message = match sub.next_message().await {
            WaitResult::Message(m) => m,
            WaitResult::Lagged(n) => {
                defmt::warn!(
                    "desktop_render: subscriber lagged, dropped {=u64} message(s)",
                    n
                );
                continue;
            }
        };
        match message {
            Inbound::Snapshot(snap) => apply_snapshot(&snap, &mut last_emotion),
            Inbound::Cmd(Cmd::Owner { name }) => {
                defmt::info!("desktop_render: owner = {=str}", name.as_str());
            }
            Inbound::Cmd(other) => {
                defmt::trace!(
                    "desktop_render: cmd received (handled by other task) {}",
                    defmt::Debug2Format(&other),
                );
            }
            Inbound::Turn(_) | Inbound::TimeSync { .. } => {
                // Out of scope for the snapshot renderer; other
                // subscribers (or a later slice) handle these.
            }
        }
    }
}

/// Translate one heartbeat snapshot into emotion + toast updates.
fn apply_snapshot(snap: &Snapshot, last_emotion: &mut Option<Emotion>) {
    let derived = derive_emotion(snap);

    match derived {
        Some(emotion) => {
            // Re-assert on every snapshot so the hold timer keeps
            // refreshing while the desktop stays engaged. Only log
            // on edge transitions to avoid spamming defmt at 10 Hz.
            if *last_emotion != Some(emotion) {
                defmt::info!("desktop_render: emotion → {=str}", emotion.wire_str(),);
                *last_emotion = Some(emotion);
            }
            REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::SetEmotion {
                emotion,
                hold_ms: EMOTION_HOLD_MS,
            });
        }
        None => {
            if last_emotion.is_some() {
                defmt::info!("desktop_render: idle (total=0); releasing autonomy");
                REMOTE_COMMAND_SIGNAL.signal(RemoteCommand::Reset);
                *last_emotion = None;
            }
        }
    }

    if !snap.msg.is_empty() {
        // `msg` may exceed the toast band's char cap; `toast::push`
        // truncates by codepoints, so passing the full string is
        // safe across multi-byte UTF-8. The toast tier stays at
        // `Warn` for both prompts and progress — the desktop only
        // sends `msg` when there's something operator-facing to
        // show, and a band that flickers between Warn/Error
        // colors per-tick would read as instability.
        toast::push(ToastLevel::Warn, &snap.msg, HalClock.now());
    }
}

/// Map heartbeat counts to an [`Emotion`].
///
/// - `waiting > 0` (a permission prompt is blocking a session) →
///   `Curious` so the operator notices something needs attention
/// - `running > 0` (work in flight, no prompt) → `Happy` so the
///   avatar reads as engaged-but-busy
/// - `total == 0` → no override (caller releases autonomy)
const fn derive_emotion(snap: &Snapshot) -> Option<Emotion> {
    if snap.waiting > 0 {
        Some(Emotion::Curious)
    } else if snap.running > 0 {
        Some(Emotion::Happy)
    } else if snap.total > 0 {
        Some(Emotion::Neutral)
    } else {
        None
    }
}
