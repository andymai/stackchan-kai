//! Networking surface for the Stack-chan firmware.
//!
//! Holds the Wi-Fi station task and the link-state signal that
//! downstream consumers (SNTP, HTTP, mDNS) wait on. Boot order:
//! avatar tasks first, then `wifi_task` — the avatar must remain
//! responsive even when there's no SSID configured or the AP is
//! unreachable.
//!
//! [`esp_now`] runs alongside the Wi-Fi stack, claiming the
//! `interfaces.esp_now` handle from `esp_radio::wifi::new`. The task
//! is inert when the operator hasn't enabled it in
//! `STACKCHAN.RON`.

pub mod esp_now;
pub mod http;
pub mod mdns;
pub mod mdns_follower;
mod respond;
pub mod snapshot;
pub mod sntp;
pub mod stack;
pub mod websocket;
pub mod wifi;
