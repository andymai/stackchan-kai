//! Wi-Fi station-mode task with offline-first retry.
//!
//! Owns the [`WifiController`] for the duration of the firmware run.
//! On boot, checks the SSID from [`stackchan_net::WifiConfig`] — an
//! empty value means "no Wi-Fi configured", and the task parks
//! silently so the avatar runs offline-first without any retry storm.
//! With a non-empty SSID it configures station mode, attempts to
//! connect, and on disconnection retries with exponential backoff.

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController, WifiEvent};

/// Public link-state signal — downstream tasks (SNTP, HTTP, mDNS)
/// `wait()` on it to gate their own startup against the connect.
///
/// `Signal` is single-consumer: it stores one waker, so multiple
/// concurrent `.wait().await` callers will lose all but the
/// last-registered. SNTP and mDNS each have a single instance and
/// loop on the signal, so this works for them. Multi-instance
/// consumers (the HTTP worker pool) must use [`LINK_READY`] instead.
pub static WIFI_LINK_SIGNAL: Signal<CriticalSectionRawMutex, WifiLinkState> = Signal::new();

/// Latched "link has been up at least once" flag.
///
/// Set the first time [`wifi_task`] observes a [`WifiLinkState::Connected`]
/// transition; never cleared. Multiple consumers can read this
/// concurrently without the single-waker contention that
/// [`WIFI_LINK_SIGNAL`] has — used by the HTTP worker pool to gate
/// each worker's first accept call.
pub static LINK_READY: AtomicBool = AtomicBool::new(false);

/// Coarse-grained Wi-Fi link state. Published on every transition.
#[derive(Clone, Copy, Debug, defmt::Format)]
pub enum WifiLinkState {
    /// No SSID configured, or the controller is currently between
    /// connect attempts.
    Disconnected,
    /// Connect attempt in progress.
    Connecting,
    /// Joined the AP. The DHCP lease + IP assignment happen in the
    /// embassy-net runner task once the link is up.
    Connected,
}

/// Backoff schedule for connect retries. Saturates at the last entry.
const RETRY_BACKOFF_MS: [u64; 5] = [1_000, 2_000, 5_000, 10_000, 30_000];

/// Wi-Fi station task entry point. Owns the [`WifiController`] for
/// the firmware lifetime. Empty SSID parks silently — the avatar
/// runs offline-first and downstream `WIFI_LINK_SIGNAL` consumers
/// see `Disconnected` and `wait()` indefinitely.
///
/// The task never returns: the embassy-net runner shares a
/// `WifiDevice` borrowed from the same controller, and dropping the
/// controller would null its backing radio state. On any setup
/// error we log and park rather than `return`.
#[embassy_executor::task]
pub async fn wifi_task(mut controller: WifiController<'static>, ssid: String, psk: String) -> ! {
    if ssid.trim().is_empty() {
        defmt::info!("wifi: no SSID configured, idle");
        WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
        park_forever().await;
    }

    let client_cfg = ClientConfig::default()
        .with_ssid(ssid.clone())
        .with_password(psk);
    if let Err(e) = controller.set_config(&ModeConfig::Client(client_cfg)) {
        defmt::error!(
            "wifi: set_config rejected ({}); parking",
            defmt::Debug2Format(&e)
        );
        park_forever().await;
    }
    if let Err(e) = controller.start_async().await {
        defmt::error!(
            "wifi: start_async failed ({}); parking",
            defmt::Debug2Format(&e)
        );
        park_forever().await;
    }

    connect_loop(controller, ssid).await
}

/// Sleeps forever without dropping the caller's stack — used by
/// `wifi_task`'s error / no-SSID paths. The embassy-net runner
/// borrows the `WifiDevice` from the controller this task owns, so
/// returning would null the runner's backing state and trip a
/// `LoadProhibited` exception.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Connect / reconnect loop with exponential backoff. Owns the
/// controller for the firmware lifetime.
async fn connect_loop(mut controller: WifiController<'static>, ssid: String) -> ! {
    let mut backoff_idx: usize = 0;
    loop {
        defmt::info!("wifi: connecting to ssid={=str}", ssid.as_str());
        WIFI_LINK_SIGNAL.signal(WifiLinkState::Connecting);
        match controller.connect_async().await {
            Ok(()) => {
                defmt::info!("wifi: connected");
                WIFI_LINK_SIGNAL.signal(WifiLinkState::Connected);
                LINK_READY.store(true, Ordering::Release);
                backoff_idx = 0;
                // Park until the controller reports a disconnect.
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                defmt::warn!("wifi: link dropped; reconnecting");
                WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
            }
            Err(e) => {
                let backoff_ms = RETRY_BACKOFF_MS[backoff_idx];
                defmt::warn!(
                    "wifi: connect failed ({}); retry in {=u64} ms",
                    defmt::Debug2Format(&e),
                    backoff_ms
                );
                WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
                Timer::after(Duration::from_millis(backoff_ms)).await;
                if backoff_idx + 1 < RETRY_BACKOFF_MS.len() {
                    backoff_idx += 1;
                }
            }
        }
    }
}
