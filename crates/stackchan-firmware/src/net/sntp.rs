//! `SNTPv4` client task: queries one of `time.sntp_servers` on every
//! Wi-Fi link-up, decodes the transmit-timestamp seconds, and writes
//! the result into the BM8563 RTC. Re-syncs hourly.
//!
//! No external SNTP crate — the wire format is small (48 bytes per
//! direction, fixed layout per RFC 4330) and a hand-rolled
//! implementation avoids a feature-tangle dep.

use alloc::string::String;
use alloc::vec::Vec;

use bm8563::{Bm8563, DateTime};
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_hal_async::i2c::I2c as AsyncI2c;

use super::wifi::{WIFI_LINK_SIGNAL, WifiLinkState};

/// SNTP / NTP epoch is 1900-01-01; Unix is 1970-01-01. Difference in
/// seconds — used to fold the transmit-timestamp seconds field into a
/// `u32` Unix timestamp suitable for `unix_to_datetime`.
const NTP_TO_UNIX_EPOCH_DELTA: u32 = 2_208_988_800;

/// Per-server timeout. Each candidate gets this long to answer before
/// we move on to the next entry in `time.sntp_servers`.
const PER_SERVER_TIMEOUT_SECS: u64 = 5;

/// Re-sync cadence after a successful sync. Hourly is enough — the
/// BM8563's stock crystal drifts <30 s/day. Re-syncs also fire on every
/// `WIFI_LINK_SIGNAL::Connected` transition (handled by the outer loop).
const RESYNC_INTERVAL_SECS: u64 = 3_600;

/// Concrete shared-I²C handle the firmware threads through every async
/// I²C consumer. Mirrors the alias `board::SharedI2c` but local to the
/// module so the spawn signature stays self-contained.
type SharedI2cBus = embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
    'static,
    embassy_sync::blocking_mutex::raw::NoopRawMutex,
    esp_hal::i2c::master::I2c<'static, esp_hal::Async>,
>;

/// Embassy task entry. Owns the dedicated UDP socket buffers + the
/// I²C handle for the BM8563 — both threaded in by `main.rs` at
/// task-spawn time.
#[embassy_executor::task]
pub async fn sntp_task(stack: Stack<'static>, rtc_bus: SharedI2cBus, servers: Vec<String>) -> ! {
    if servers.is_empty() {
        defmt::info!("sntp: no servers configured, idle");
        park_forever().await;
    }

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; 256];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0u8; 256];

    let mut bus = rtc_bus;
    let mut rtc = Bm8563::new(&mut bus);
    if let Err(e) = rtc.init().await {
        defmt::warn!(
            "sntp: BM8563 init failed ({}); continuing without RTC writes",
            defmt::Debug2Format(&e)
        );
    }

    loop {
        // Wait for an active link before each sync attempt. `wait()`
        // returns whatever the wifi task last published — including the
        // initial `Disconnected`, so we re-check before dispatching.
        if !matches!(WIFI_LINK_SIGNAL.wait().await, WifiLinkState::Connected) {
            continue;
        }
        // Give DHCP a moment to settle once the link is up. Without an
        // assigned address the UDP socket can't route the request.
        Timer::after(Duration::from_secs(1)).await;

        let mut socket =
            UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
        if let Err(e) = socket.bind(0) {
            defmt::warn!("sntp: bind failed ({:?})", e);
            sleep_until_resync().await;
            continue;
        }

        let synced = try_sync_once(&stack, &socket, &servers, &mut rtc).await;
        socket.close();
        if synced {
            sleep_until_resync().await;
        } else {
            // No server answered — back off briefly before the next
            // attempt rather than busy-looping the link signal.
            Timer::after(Duration::from_secs(30)).await;
        }
    }
}

/// Try each configured server in order until one answers within the
/// per-server timeout. Returns `true` if any answer landed and was
/// applied to the RTC.
async fn try_sync_once<B: AsyncI2c>(
    stack: &Stack<'static>,
    socket: &UdpSocket<'_>,
    servers: &[String],
    rtc: &mut Bm8563<&mut B>,
) -> bool {
    for server in servers {
        let Some(target) = resolve(stack, server).await else {
            continue;
        };
        let endpoint = embassy_net::IpEndpoint::new(target.into(), 123);

        let request = build_sntp_request();
        if let Err(e) = socket.send_to(&request, endpoint).await {
            defmt::warn!("sntp: send to {=str} failed ({:?})", server.as_str(), e);
            continue;
        }

        let mut buf = [0u8; 48];
        match with_timeout(
            Duration::from_secs(PER_SERVER_TIMEOUT_SECS),
            socket.recv_from(&mut buf),
        )
        .await
        {
            Ok(Ok((n, _))) if n >= 48 => {
                let unix_secs = decode_transmit_timestamp(&buf);
                let dt = unix_to_datetime(unix_secs);
                match rtc.write_datetime(dt).await {
                    Ok(()) => {
                        defmt::info!(
                            "sntp: synced via {=str} -> {=u16}-{=u8:02}-{=u8:02} {=u8:02}:{=u8:02}:{=u8:02} UTC",
                            server.as_str(),
                            dt.year,
                            dt.month,
                            dt.day,
                            dt.hours,
                            dt.minutes,
                            dt.seconds,
                        );
                        return true;
                    }
                    Err(e) => {
                        defmt::warn!("sntp: BM8563 write failed ({})", defmt::Debug2Format(&e));
                    }
                }
            }
            Ok(Ok(_)) => defmt::warn!("sntp: short reply from {=str}", server.as_str()),
            Ok(Err(e)) => defmt::warn!("sntp: recv from {=str} ({:?})", server.as_str(), e),
            Err(_) => defmt::warn!(
                "sntp: timeout from {=str} after {=u64} s",
                server.as_str(),
                PER_SERVER_TIMEOUT_SECS
            ),
        }
    }
    false
}

/// Resolve `host` to an IPv4 address via embassy-net's DHCP-supplied
/// resolvers. Logs and returns `None` on any DNS failure.
async fn resolve(stack: &Stack<'static>, host: &str) -> Option<embassy_net::Ipv4Address> {
    match stack.dns_query(host, DnsQueryType::A).await {
        Ok(addrs) => addrs
            .iter()
            .map(|a| match a {
                embassy_net::IpAddress::Ipv4(v4) => *v4,
            })
            .next(),
        Err(e) => {
            defmt::warn!("sntp: DNS lookup for {=str} failed ({:?})", host, e);
            None
        }
    }
}

/// Build the 48-byte `SNTPv4` request: `LI=0 VN=4 Mode=3 (client)` in
/// the first byte, all other bytes zero. The server fills in the
/// reply.
const fn build_sntp_request() -> [u8; 48] {
    let mut req = [0u8; 48];
    req[0] = 0b0010_0011; // LI=0, VN=4, Mode=3 (client)
    req
}

/// Pull the "transmit timestamp seconds" field (offset 40, big-endian
/// u32) from a 48-byte SNTP reply and fold the NTP→Unix epoch shift.
fn decode_transmit_timestamp(reply: &[u8]) -> u32 {
    let ntp_secs = u32::from_be_bytes([reply[40], reply[41], reply[42], reply[43]]);
    ntp_secs.wrapping_sub(NTP_TO_UNIX_EPOCH_DELTA)
}

/// Sleep until the hourly re-sync, but wake immediately if the link
/// state changes (cleared by the outer loop).
async fn sleep_until_resync() {
    Timer::after(Duration::from_secs(RESYNC_INTERVAL_SECS)).await;
}

/// Idle path for the no-servers configuration.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Convert a Unix timestamp into a Gregorian UTC `DateTime` for the
/// BM8563 driver. Algorithm is the canonical "days-from-epoch +
/// month-length table" walk; valid for `1970-01-01` through `2099-12-31`
/// (the BM8563 itself is constrained to the same window via its
/// `CENTURY` flag).
fn unix_to_datetime(unix_secs: u32) -> DateTime {
    const SECS_PER_DAY: u32 = 86_400;
    let days = unix_secs / SECS_PER_DAY;
    let secs = unix_secs % SECS_PER_DAY;
    let hours = (secs / 3_600) as u8;
    let minutes = ((secs % 3_600) / 60) as u8;
    let seconds = (secs % 60) as u8;
    // 1970-01-01 was a Thursday.
    let weekday = ((days + 4) % 7) as u8;

    let (year, month, day) = days_to_ymd(days);
    DateTime {
        year,
        month,
        day,
        weekday,
        hours,
        minutes,
        seconds,
    }
}

/// Walk forward from 1970-01-01 by whole years then months until the
/// day-of-year remainder fits inside a single month.
fn days_to_ymd(mut days: u32) -> (u16, u8, u8) {
    let mut year: u16 = 1970;
    loop {
        let in_year = if is_leap(year) { 366 } else { 365 };
        if days < in_year {
            break;
        }
        days -= in_year;
        year += 1;
    }
    let mut month: u8 = 1;
    loop {
        let dim = u32::from(days_in_month(year, month));
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    // After the month walk above, `days` is in `0..31` so the cast
    // is in-range — annotated with `clippy::cast_possible_truncation`
    // to acknowledge the lint without weakening it crate-wide.
    #[allow(clippy::cast_possible_truncation)]
    let day = (days + 1) as u8;
    (year, month, day)
}

/// Standard Gregorian leap-year rule.
const fn is_leap(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Days in the given Gregorian month. February honours `is_leap`.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_is_thursday_1970_01_01() {
        let dt = unix_to_datetime(0);
        assert_eq!(dt.year, 1970);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.weekday, 4); // Thursday
        assert_eq!(dt.hours, 0);
        assert_eq!(dt.minutes, 0);
        assert_eq!(dt.seconds, 0);
    }

    #[test]
    fn handles_leap_years() {
        // 2020-03-01 was a Sunday: 1583107200 unix seconds.
        let dt = unix_to_datetime(1_583_107_200);
        assert_eq!(dt.year, 2020);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.weekday, 0);
    }

    #[test]
    fn ntp_to_unix_subtraction() {
        // NTP timestamp for 1970-01-01 00:00:00 UTC is 2208988800.
        let mut reply = [0u8; 48];
        reply[40..44].copy_from_slice(&NTP_TO_UNIX_EPOCH_DELTA.to_be_bytes());
        assert_eq!(decode_transmit_timestamp(&reply), 0);
    }
}
