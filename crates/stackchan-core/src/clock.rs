//! Monotonic clock abstraction.
//!
//! A [`Clock`] is a source of monotonically-non-decreasing [`Instant`]s with
//! millisecond resolution. Firmware implementations wrap embassy-time; sim
//! implementations advance a counter manually.

use core::ops::{Add, Sub};

/// A point in time with millisecond resolution.
///
/// `Instant` is always monotonically non-decreasing within a single
/// [`Clock`] implementation. Cross-clock comparisons are not meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant {
    /// Milliseconds since the clock epoch.
    millis: u64,
}

impl Instant {
    /// Zero instant -- the beginning of the clock epoch.
    pub const ZERO: Self = Self { millis: 0 };

    /// Construct an `Instant` from milliseconds since the clock epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    /// Milliseconds since the clock epoch.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.millis
    }

    /// Saturating subtraction; returns the non-negative `Duration` between
    /// `self` and `earlier` (in milliseconds). If `earlier` is in the future
    /// relative to `self`, returns zero.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> u64 {
        self.millis.saturating_sub(earlier.millis)
    }
}

impl Add<u64> for Instant {
    type Output = Self;

    /// Adds a duration in milliseconds.
    fn add(self, millis: u64) -> Self {
        Self {
            millis: self.millis.saturating_add(millis),
        }
    }
}

impl Sub for Instant {
    type Output = u64;

    /// Saturating subtraction in milliseconds.
    fn sub(self, rhs: Self) -> u64 {
        self.saturating_duration_since(rhs)
    }
}

/// Monotonic clock trait. Implementations must guarantee that successive
/// calls to [`Clock::now`] return non-decreasing `Instant` values.
pub trait Clock {
    /// Read the current monotonic time.
    fn now(&self) -> Instant;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_ordering() {
        assert!(Instant::from_millis(10) > Instant::from_millis(5));
        assert_eq!(Instant::ZERO.as_millis(), 0);
    }

    #[test]
    fn saturating_duration() {
        let a = Instant::from_millis(100);
        let b = Instant::from_millis(40);
        assert_eq!(a.saturating_duration_since(b), 60);
        // Reverse order saturates to zero.
        assert_eq!(b.saturating_duration_since(a), 0);
    }

    #[test]
    fn instant_add_saturates() {
        let end = Instant::from_millis(u64::MAX - 10);
        let after = end + 1000;
        assert_eq!(after.as_millis(), u64::MAX);
    }
}
