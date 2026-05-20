//! Hourly chime task — a short chirp at every wall-clock top-of-hour.
//!
//! Off by default. Enabled via `behavior.hourly_chime_enabled` in
//! `STACKCHAN.RON` or `PUT /settings`. The task waits for an
//! SNTP-synced wall clock before scheduling, then loops:
//!
//! 1. Read the current wall time from the BM8563 RTC.
//! 2. Compute milliseconds remaining until the next top-of-hour.
//! 3. Sleep that long.
//! 4. Re-read the clock to handle skew across the sleep, and emit a
//!    [`stackchan_core::PhraseId::WakeChirp`] (~100 ms 1 kHz sine)
//!    via [`crate::net::http::REMOTE_COMMAND_QUEUE`] if we landed
//!    inside the per-hour acceptance window.
//!
//! ## Why poll instead of an absolute timer
//!
//! `embassy_time::Timer::at` can't reach across an SNTP-driven wall
//! clock — its instants are monotonic (boot-relative). The task
//! recomputes the next-fire delay from the live RTC each loop so a
//! mid-day clock correction (DST shift, late SNTP sync, manual nudge)
//! corrects on the very next iteration.

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_time::{Duration, Timer};
use stackchan_core::RemoteCommand;
use stackchan_core::voice::{Locale, PhraseId, Priority};

use crate::board::SharedI2cBus;
use crate::net::http::enqueue_remote_command;

/// Acceptance window around the top-of-hour. Sleep math + RTC read
/// jitter can drift the wakeup by a few hundred milliseconds; if the
/// landed minute is `00` we accept regardless, otherwise the window
/// guards against double-firing if the loop wakes a hair before
/// rollover.
const TOP_OF_HOUR_WINDOW_SECS: u8 = 5;

/// Polling delay used while the wall clock is unreliable (RTC failure
/// or unset year — the BM8563 boots reading year=2000 / 1970-ish).
const WAIT_FOR_CLOCK_SECS: u64 = 30;

/// Lower bound on a parsed RTC year before the chime task trusts it.
/// The BM8563 reports `year = 2000` on power-up before SNTP runs;
/// gating the schedule on a year we know is post-firmware-release
/// avoids firing 23 chimes on the way to actual wall time.
const MIN_TRUSTED_YEAR: u16 = 2025;

/// Embassy task that emits the hourly chirp. `i2c_bus` is the shared
/// I²C handle the SNTP / wallclock task uses too — the chime task
/// only reads, never writes the RTC.
#[embassy_executor::task]
pub async fn chime_task(i2c_bus: &'static SharedI2cBus, enabled: bool) {
    if !enabled {
        defmt::info!("chime: disabled in config — task exiting");
        return;
    }
    defmt::info!(
        "chime: hourly chime enabled (window ±{=u8}s)",
        TOP_OF_HOUR_WINDOW_SECS
    );

    let mut bus = I2cDevice::new(i2c_bus);
    loop {
        let dt = match crate::wallclock::read_datetime(&mut bus).await {
            Some(dt) if dt.year >= MIN_TRUSTED_YEAR => dt,
            _ => {
                // Wall clock not yet trustworthy. Wait a bit and retry
                // — SNTP usually nails the RTC within a minute of join.
                Timer::after(Duration::from_secs(WAIT_FOR_CLOCK_SECS)).await;
                continue;
            }
        };

        let secs_until = secs_until_top_of_hour(dt.minutes, dt.seconds);
        Timer::after(Duration::from_secs(u64::from(secs_until))).await;

        // Re-read after the sleep — RTC drift, NTP corrections, or a
        // tiny over-/undershoot from the embassy timer can land us
        // a couple of seconds off.
        let Some(post) = crate::wallclock::read_datetime(&mut bus).await else {
            defmt::warn!("chime: post-sleep RTC read failed; will resync");
            continue;
        };
        if !is_within_top_of_hour_window(post.minutes, post.seconds) {
            defmt::warn!(
                "chime: woke off-window at {=u8:02}:{=u8:02}:{=u8:02}; resyncing",
                post.hours,
                post.minutes,
                post.seconds,
            );
            continue;
        }

        defmt::info!("chime: hour {=u8:02}:00 — emitting WakeChirp", post.hours);
        enqueue_remote_command(RemoteCommand::Speak {
            phrase: PhraseId::WakeChirp,
            locale: Locale::En,
            priority: Priority::Normal,
        });
        // Avoid a double-fire if we land slightly before :00:00 and
        // the next iteration's `secs_until_top_of_hour` returns 0.
        Timer::after(Duration::from_secs(u64::from(TOP_OF_HOUR_WINDOW_SECS) + 1)).await;
    }
}

/// Compute seconds remaining until the next `mm=00, ss=00`. If we're
/// already at `00:00`, returns 3600 (skip to the next hour rather
/// than firing twice in the same minute).
#[must_use]
pub const fn secs_until_top_of_hour(minutes: u8, seconds: u8) -> u32 {
    let m = minutes as u32;
    let s = seconds as u32;
    if m == 0 && s == 0 {
        return 3600;
    }
    // Seconds elapsed in the current hour, then the complement.
    let elapsed = m * 60 + s;
    3600 - elapsed
}

/// Check whether `(mm, ss)` is within `TOP_OF_HOUR_WINDOW_SECS` of
/// the top of an hour (either side).
///
/// Used to suppress a fire when the wall clock has drifted away from
/// `mm=00` between scheduling and waking.
#[must_use]
pub const fn is_within_top_of_hour_window(minutes: u8, seconds: u8) -> bool {
    if minutes == 0 && seconds <= TOP_OF_HOUR_WINDOW_SECS {
        return true;
    }
    if minutes == 59 && seconds >= 60_u8.saturating_sub(TOP_OF_HOUR_WINDOW_SECS) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secs_until_zero_when_at_top_returns_full_hour() {
        // Exactly `:00:00` should not fire immediately — return
        // a full hour so the very next loop iteration sleeps until
        // the next top-of-hour rather than re-firing in-place.
        assert_eq!(secs_until_top_of_hour(0, 0), 3600);
    }

    #[test]
    fn secs_until_handles_partial_minute() {
        // `:30:15` → 29*60 + 45 = 1785s remaining.
        assert_eq!(secs_until_top_of_hour(30, 15), 29 * 60 + 45);
    }

    #[test]
    fn secs_until_handles_late_in_hour() {
        // `:59:55` → 5s.
        assert_eq!(secs_until_top_of_hour(59, 55), 5);
    }

    #[test]
    fn within_window_accepts_just_before_and_after() {
        assert!(is_within_top_of_hour_window(0, 0));
        assert!(is_within_top_of_hour_window(0, TOP_OF_HOUR_WINDOW_SECS));
        assert!(is_within_top_of_hour_window(
            59,
            60 - TOP_OF_HOUR_WINDOW_SECS
        ));
    }

    #[test]
    fn within_window_rejects_mid_hour() {
        assert!(!is_within_top_of_hour_window(15, 0));
        assert!(!is_within_top_of_hour_window(45, 30));
        // Just outside the window on either side.
        assert!(!is_within_top_of_hour_window(
            0,
            TOP_OF_HOUR_WINDOW_SECS + 1
        ));
    }
}
