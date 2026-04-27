//! Networking surface for the Stack-chan firmware.
//!
//! Holds the Wi-Fi station task and the link-state signal that
//! downstream consumers (SNTP, HTTP, mDNS) wait on. Boot order:
//! avatar tasks first, then `wifi_task` — the avatar must remain
//! responsive even when there's no SSID configured or the AP is
//! unreachable.

pub mod http;
pub mod json;
pub mod mdns;
pub mod snapshot;
pub mod sntp;
pub mod stack;
pub mod wifi;
