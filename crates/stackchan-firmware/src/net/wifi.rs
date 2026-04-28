//! Wi-Fi station-mode task with offline-first retry + soft reconfig.
//!
//! Owns the [`WifiController`] for the duration of the firmware run.
//! On boot, checks the SSID from [`stackchan_net::WifiConfig`] — an
//! empty value means "no Wi-Fi configured", and the task parks on
//! [`WIFI_RECONFIG`] so a later BLE provisioning write or HTTP `PUT
//! /settings` can hand it credentials without a reboot. With a
//! non-empty SSID it configures station mode, attempts to connect,
//! and on disconnection retries with exponential backoff.
//!
//! Soft reconfig: when [`WIFI_RECONFIG`] fires (from the BLE
//! provisioning service or `PUT /settings`), the task drops the
//! current link, swaps creds, and re-enters the connect loop. The
//! HTTP server's settings handler used to return
//! `{"reboot_required": true}` for a Wi-Fi change; with this signal
//! wired up the user-visible recovery is just a brief link blip.

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
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

/// Soft-reconfig signal — replace the active Wi-Fi credentials.
///
/// Producers: HTTP `PUT /settings` (after a successful SD writeback)
/// and the BLE provisioning service (after a writable SSID/PSK
/// characteristic write). The wifi task is the single consumer; new
/// values overwrite older pending ones (latest-wins) before the task
/// actually picks them up, which is the right semantics if a user
/// retries provisioning rapidly.
pub static WIFI_RECONFIG: Signal<CriticalSectionRawMutex, WifiCreds> = Signal::new();

/// Credentials handed to [`WIFI_RECONFIG`].
#[derive(Clone, Debug)]
pub struct WifiCreds {
    /// SSID. Empty string means "park; wait for a non-empty value".
    pub ssid: String,
    /// Pre-shared key. Empty for an open AP.
    pub psk: String,
}

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
/// the firmware lifetime. Empty SSID parks on [`WIFI_RECONFIG`] —
/// the avatar runs offline-first; a later BLE provisioning write or
/// `PUT /settings` wakes the task without a reboot.
///
/// The task never returns: the embassy-net runner shares a
/// `WifiDevice` borrowed from the same controller, and dropping the
/// controller would null its backing radio state. On any setup
/// error we log and park rather than `return`.
#[embassy_executor::task]
pub async fn wifi_task(
    mut controller: WifiController<'static>,
    initial_ssid: String,
    initial_psk: String,
) -> ! {
    let mut creds = WifiCreds {
        ssid: initial_ssid,
        psk: initial_psk,
    };
    // `controller.start_async()` is one-shot per radio lifetime. Track
    // whether we've called it so a fresh boot with an empty SSID
    // doesn't start the radio until the first valid creds arrive
    // (saving idle airtime), and a re-provisioning doesn't try to
    // start it twice.
    let mut started = false;

    loop {
        if creds.ssid.trim().is_empty() {
            defmt::info!("wifi: no SSID configured, idle");
            WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
            creds = WIFI_RECONFIG.wait().await;
            continue;
        }

        let client_cfg = ClientConfig::default()
            .with_ssid(creds.ssid.clone())
            .with_password(creds.psk.clone());
        if let Err(e) = controller.set_config(&ModeConfig::Client(client_cfg)) {
            defmt::error!(
                "wifi: set_config rejected ({}); waiting for next provisioning",
                defmt::Debug2Format(&e)
            );
            WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
            creds = WIFI_RECONFIG.wait().await;
            continue;
        }
        if !started {
            if let Err(e) = controller.start_async().await {
                defmt::error!(
                    "wifi: start_async failed ({}); parking",
                    defmt::Debug2Format(&e)
                );
                park_forever().await;
            }
            started = true;
        }

        let next_creds = run_connect_loop(&mut controller, creds.ssid.as_str()).await;
        creds = next_creds;
    }
}

/// Sleeps forever without dropping the caller's stack — used by
/// `wifi_task`'s unrecoverable error paths (where `start_async`
/// itself failed). The embassy-net runner borrows the `WifiDevice`
/// from the controller this task owns, so returning would null the
/// runner's backing state and trip a `LoadProhibited` exception.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Connect / reconnect loop with exponential backoff *and* reconfig
/// handling. Returns the new credentials when [`WIFI_RECONFIG`] fires;
/// the caller (`wifi_task`) re-applies them via `set_config` and
/// re-enters this loop.
async fn run_connect_loop(controller: &mut WifiController<'static>, ssid: &str) -> WifiCreds {
    let mut backoff_idx: usize = 0;
    loop {
        defmt::info!("wifi: connecting to ssid={=str}", ssid);
        WIFI_LINK_SIGNAL.signal(WifiLinkState::Connecting);

        let connect_outcome = select(controller.connect_async(), WIFI_RECONFIG.wait()).await;
        match connect_outcome {
            Either::First(Ok(())) => {
                defmt::info!("wifi: connected");
                WIFI_LINK_SIGNAL.signal(WifiLinkState::Connected);
                LINK_READY.store(true, Ordering::Release);
                backoff_idx = 0;

                let live_outcome = select(
                    controller.wait_for_event(WifiEvent::StaDisconnected),
                    WIFI_RECONFIG.wait(),
                )
                .await;
                match live_outcome {
                    Either::First(()) => {
                        defmt::warn!("wifi: link dropped; reconnecting");
                        WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
                    }
                    Either::Second(new_creds) => {
                        defmt::info!(
                            "wifi: reconfig while connected (new ssid={=str})",
                            new_creds.ssid.as_str()
                        );
                        let _ = controller.disconnect_async().await;
                        WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);
                        return new_creds;
                    }
                }
            }
            Either::First(Err(e)) => {
                let backoff_ms = RETRY_BACKOFF_MS[backoff_idx];
                defmt::warn!(
                    "wifi: connect failed ({}); retry in {=u64} ms",
                    defmt::Debug2Format(&e),
                    backoff_ms
                );
                WIFI_LINK_SIGNAL.signal(WifiLinkState::Disconnected);

                let wait_outcome = select(
                    Timer::after(Duration::from_millis(backoff_ms)),
                    WIFI_RECONFIG.wait(),
                )
                .await;
                match wait_outcome {
                    Either::First(()) => {
                        if backoff_idx + 1 < RETRY_BACKOFF_MS.len() {
                            backoff_idx += 1;
                        }
                    }
                    Either::Second(new_creds) => {
                        defmt::info!(
                            "wifi: reconfig during backoff (new ssid={=str})",
                            new_creds.ssid.as_str()
                        );
                        return new_creds;
                    }
                }
            }
            Either::Second(new_creds) => {
                defmt::info!(
                    "wifi: reconfig during connect attempt (new ssid={=str})",
                    new_creds.ssid.as_str()
                );
                // The connect attempt may still be in flight in the
                // radio; the outer loop's `set_config` + `connect_async`
                // supersede it. A speculative `disconnect_async` here
                // would block on a `StaDisconnected` event the radio
                // may never emit (we never reached `Connected`).
                return new_creds;
            }
        }
    }
}
