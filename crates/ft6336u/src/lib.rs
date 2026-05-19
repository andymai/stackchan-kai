//! # ft6336u
//!
//! `no_std` async driver for the `FocalTech` FT6336U capacitive touch
//! controller. Supports the single-touch subset of the chip's
//! functionality — one `read_touch()` returns the current finger
//! count + first touch coordinates. Multi-touch, native gesture
//! registers, and power-saving modes are intentionally out of scope
//! for the MVP; they can be added in place without changing the
//! `Ft6336u` type surface.
//!
//! The driver is generic over any [`embedded_hal_async::i2c::I2c`] so
//! it plugs into either a directly-owned bus or a shared-bus wrapper
//! (e.g. `embassy-embedded-hal`'s `I2cDevice`).
//!
//! ## CoreS3 wiring
//!
//! The FT6336U sits on the internal I²C0 (SCL = GPIO11, SDA = GPIO12)
//! at address [`CORES3_ADDRESS`]. Reset is handled by the AW9523 IO
//! expander as part of the LCD bring-up, so this driver assumes the
//! chip is already out of reset by the time [`Ft6336u::new`] runs.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::i2c::I2c;
//! # async fn demo<B: I2c>(bus: B) -> Result<(), ft6336u::Error<B::Error>> {
//! let mut touch = ft6336u::Ft6336u::new(bus);
//! let vendor = touch.read_vendor_id().await?;
//! assert_eq!(vendor, ft6336u::VENDOR_ID_FOCALTECH);
//!
//! if let Some(report) = touch.read_touch().await?.point() {
//!     let (x, y) = report;
//!     // ... do something with the touch position ...
//!     let _ = (x, y);
//! }
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::i2c::I2c;

/// 7-bit I²C address of the FT6336U on CoreS3.
pub const CORES3_ADDRESS: u8 = 0x38;

/// Vendor ID value read back from register `0xA8` for genuine
/// `FocalTech` FT6336 family parts. Used as a cheap presence / identity
/// check at boot.
pub const VENDOR_ID_FOCALTECH: u8 = 0x11;

/// Device operating mode register. Reset value: `0x00`
/// (interrupt-on-change mode). Not written by this driver; included so
/// a single bulk read from `REG_G_MODE` covers the status + first
/// touch-point in one transaction.
const REG_G_MODE: u8 = 0x00;

/// Vendor-ID register. Reads back [`VENDOR_ID_FOCALTECH`] on genuine
/// parts.
const REG_VENDOR_ID: u8 = 0xA8;

/// Width of the bulk touch-status read in bytes:
///
/// | Offset | Register    | Contents                              |
/// |--------|-------------|---------------------------------------|
/// | 0      | `G_MODE`    | Operating mode (unused here)          |
/// | 1      | `GESTURE`   | HW-detected gesture code (unused)     |
/// | 2      | `TD_STATUS` | Low nibble = touch count              |
/// | 3      | `P1_XH`     | `[7:6]` event flag, `[3:0]` x\[11:8\] |
/// | 4      | `P1_XL`     | x\[7:0\]                              |
/// | 5      | `P1_YH`     | `[7:4]` touch id, `[3:0]` y\[11:8\]   |
/// | 6      | `P1_YL`     | y\[7:0\]                              |
const TOUCH_READ_LEN: usize = 7;

/// Offset of `TD_STATUS` inside the [`TOUCH_READ_LEN`]-byte window.
const OFFSET_TD_STATUS: usize = 2;
/// Offset of `P1_XH` (first touch point, high nibble of X).
const OFFSET_P1_XH: usize = 3;
/// Offset of `P1_XL` (first touch point, low byte of X).
const OFFSET_P1_XL: usize = 4;
/// Offset of `P1_YH` (first touch point, high nibble of Y).
const OFFSET_P1_YH: usize = 5;
/// Offset of `P1_YL` (first touch point, low byte of Y).
const OFFSET_P1_YL: usize = 6;

/// Mask used to extract the 12-bit X / Y coordinate's high nibble from
/// `P1_XH` / `P1_YH`. The top two bits of `P1_XH` carry the event flag
/// and the top nibble of `P1_YH` carries the touch ID; both are
/// ignored in the MVP.
const COORD_HIGH_MASK: u8 = 0x0F;

/// Mask used to extract the touch-point count from `TD_STATUS`.
const TD_STATUS_COUNT_MASK: u8 = 0x0F;

/// Driver error type.
///
/// Single variant today; kept as a `non_exhaustive` enum so future
/// additions (e.g. a dedicated `ChipIdMismatch` variant) won't be
/// breaking.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// One decoded FT6336U touch status snapshot.
///
/// The datasheet reports 0, 1, or 2 simultaneous fingers in the low
/// nibble of `TD_STATUS`. This driver only exposes the first point;
/// `fingers` is still reported verbatim so callers can distinguish
/// "one finger" from "two fingers" without us needing to grow the
/// API surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TouchReport {
    /// Number of fingers currently in contact with the panel
    /// (0..=2 on FT6336U).
    pub fingers: u8,
    /// Coordinates of the first touch point, or `None` if no fingers
    /// are down. X/Y are in the panel's native 12-bit coordinate
    /// space; depending on panel orientation, callers may need to
    /// swap or invert axes to match the framebuffer.
    pub first: Option<(u16, u16)>,
}

impl TouchReport {
    /// Convenience: `true` iff one or more fingers are currently down.
    #[must_use]
    pub const fn is_touched(&self) -> bool {
        self.fingers > 0
    }

    /// Coordinates of the first touch point, if any. Equivalent to
    /// [`TouchReport::first`] — named for ergonomics at call sites.
    #[must_use]
    pub const fn point(&self) -> Option<(u16, u16)> {
        self.first
    }
}

/// FT6336U driver.
///
/// Generic over any async I²C bus that implements
/// [`embedded_hal_async::i2c::I2c`]. Does not cache any state — every
/// call hits the bus, so the chip's internal timing (polling rate
/// ≤ 60 Hz for controller-limited reasons) is the only thing bounding
/// caller cadence.
pub struct Ft6336u<B> {
    /// The I²C bus the chip is addressed on.
    bus: B,
}

impl<B: I2c> Ft6336u<B> {
    /// Wrap an I²C bus already configured for the FT6336U's 400 kHz
    /// max rate (100 kHz is also fine, as the driver does nothing
    /// rate-sensitive).
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Read the vendor-ID register. Returns [`VENDOR_ID_FOCALTECH`]
    /// (`0x11`) on genuine FT6336 family parts. Useful as a presence
    /// check at boot.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error encountered during the register
    /// read.
    pub async fn read_vendor_id(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8];
        self.bus
            .write_read(CORES3_ADDRESS, &[REG_VENDOR_ID], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Poll the touch-status registers and return a decoded snapshot.
    ///
    /// Always issues a single 7-byte read starting at register `0x00`
    /// (`G_MODE`). Empty panels (zero fingers) still cost one
    /// transaction — no short-circuit is possible without losing
    /// touch-release detection.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error. A chip that returns all-ones
    /// bytes (common failure mode for unpowered parts) decodes as
    /// `fingers = 15`, which callers can filter as a sanity check if
    /// desired.
    pub async fn read_touch(&mut self) -> Result<TouchReport, Error<B::Error>> {
        let mut buf = [0u8; TOUCH_READ_LEN];
        self.bus
            .write_read(CORES3_ADDRESS, &[REG_G_MODE], &mut buf)
            .await?;
        Ok(decode_touch(buf))
    }

    /// Drop the driver, returning the wrapped bus so the caller can
    /// share it with another device (or explicitly release it).
    #[must_use]
    pub fn into_inner(self) -> B {
        self.bus
    }
}

/// Decode the 7-byte bulk read into a [`TouchReport`]. Separated so
/// unit tests can exercise the bit-twiddling without a real bus.
///
/// Takes the buffer by value: clippy prefers this for arrays ≤ 8 bytes
/// since the copy is cheaper than indirecting through a reference.
fn decode_touch(buf: [u8; TOUCH_READ_LEN]) -> TouchReport {
    let fingers = buf[OFFSET_TD_STATUS] & TD_STATUS_COUNT_MASK;
    let first = if fingers == 0 {
        None
    } else {
        let xh = u16::from(buf[OFFSET_P1_XH] & COORD_HIGH_MASK);
        let xl = u16::from(buf[OFFSET_P1_XL]);
        let yh = u16::from(buf[OFFSET_P1_YH] & COORD_HIGH_MASK);
        let yl = u16::from(buf[OFFSET_P1_YL]);
        Some(((xh << 8) | xl, (yh << 8) | yl))
    };
    TouchReport { fingers, first }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test scaffolding: Infallible bus error makes unwrap() sound"
)]
#[allow(
    clippy::future_not_send,
    reason = "test mocks hold RefCell for event recording; single-threaded block_on runs them"
)]
#[allow(
    clippy::panic,
    reason = "test scaffolding: mock panics if the driver issues an unexpected op shape"
)]
mod tests {
    use super::*;

    #[test]
    fn no_touch_decodes_to_empty_report() {
        let buf = [0u8; TOUCH_READ_LEN];
        let report = decode_touch(buf);
        assert_eq!(report.fingers, 0);
        assert!(report.first.is_none());
        assert!(!report.is_touched());
    }

    #[test]
    fn single_touch_decodes_coordinates() {
        // One finger at (0x123, 0x045). Event flag bits in P1_XH and
        // touch-id bits in P1_YH are set to nonzero to prove the
        // masks strip them.
        let mut buf = [0u8; TOUCH_READ_LEN];
        buf[OFFSET_TD_STATUS] = 0x01;
        buf[OFFSET_P1_XH] = 0b1100_0001; // event=11, x_high=0x1
        buf[OFFSET_P1_XL] = 0x23;
        buf[OFFSET_P1_YH] = 0b0101_0000; // touch_id=0x5, y_high=0x0
        buf[OFFSET_P1_YL] = 0x45;
        let report = decode_touch(buf);
        assert_eq!(report.fingers, 1);
        assert_eq!(report.first, Some((0x0123, 0x0045)));
        assert!(report.is_touched());
    }

    #[test]
    fn two_fingers_report_count_but_only_first_coords() {
        let mut buf = [0u8; TOUCH_READ_LEN];
        buf[OFFSET_TD_STATUS] = 0x02;
        buf[OFFSET_P1_XH] = 0x00;
        buf[OFFSET_P1_XL] = 0x10;
        buf[OFFSET_P1_YH] = 0x00;
        buf[OFFSET_P1_YL] = 0x20;
        let report = decode_touch(buf);
        assert_eq!(report.fingers, 2);
        assert_eq!(report.first, Some((0x0010, 0x0020)));
    }

    #[test]
    fn td_status_high_nibble_is_ignored() {
        let mut buf = [0u8; TOUCH_READ_LEN];
        // Datasheet leaves high nibble reserved; don't misread a
        // reserved bit as "many fingers."
        buf[OFFSET_TD_STATUS] = 0xF1;
        buf[OFFSET_P1_XL] = 0x05;
        buf[OFFSET_P1_YL] = 0x07;
        let report = decode_touch(buf);
        assert_eq!(report.fingers, 1);
        assert_eq!(report.first, Some((0x0005, 0x0007)));
    }

    #[test]
    fn max_12_bit_coordinates_decode_correctly() {
        // (0xFFF, 0xFFF) is the corner of the 12-bit coordinate space.
        // High nibble of P1_XH/P1_YH = 0xF, low byte = 0xFF. Pins the
        // shift+OR composition against a regression that confused the
        // mask (0x0F) with the multiplier (<< 8).
        let mut buf = [0u8; TOUCH_READ_LEN];
        buf[OFFSET_TD_STATUS] = 0x01;
        buf[OFFSET_P1_XH] = 0x0F;
        buf[OFFSET_P1_XL] = 0xFF;
        buf[OFFSET_P1_YH] = 0x0F;
        buf[OFFSET_P1_YL] = 0xFF;
        let report = decode_touch(buf);
        assert_eq!(report.first, Some((0x0FFF, 0x0FFF)));
    }

    #[test]
    fn all_ones_burst_decodes_to_fingers_15() {
        // Unpowered / wedged chips return 0xFF for every byte on the
        // bus. The decoder reports fingers=15 and coordinates derived
        // from the masked nibbles. The driver docs promise callers
        // can filter on fingers > 2 as a sanity check; pinning this
        // behaviour so it stays a stable failure-mode signal rather
        // than being silently normalised away.
        let buf = [0xFF; TOUCH_READ_LEN];
        let report = decode_touch(buf);
        assert_eq!(report.fingers, 15);
        // Coords are still decoded from the masked nibbles.
        assert_eq!(report.first, Some((0x0FFF, 0x0FFF)));
        assert!(report.is_touched(), "fingers > 0 still counts as touched");
    }

    #[test]
    fn touch_report_point_is_an_alias_for_first() {
        // Documented alias — pinning equivalence so a future
        // divergence (e.g. point() applies an axis swap) doesn't
        // silently happen without a corresponding test update.
        let report = TouchReport {
            fingers: 1,
            first: Some((123, 456)),
        };
        assert_eq!(report.point(), report.first);

        let empty = TouchReport::default();
        assert_eq!(empty.point(), None);
        assert!(!empty.is_touched());
    }

    // --- I²C-path coverage via the shared MockI2c pattern ---

    use core::cell::RefCell;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        WriteRead(u8, u8, usize),
    }

    struct Harness {
        events: RefCell<Vec<Event>>,
        read_responses: RefCell<Vec<Vec<u8>>>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                read_responses: RefCell::new(Vec::new()),
            }
        }

        fn queue_read(&self, bytes: Vec<u8>) {
            self.read_responses.borrow_mut().push(bytes);
        }

        fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }
    }

    struct MockI2c<'a> {
        harness: &'a Harness,
    }

    impl embedded_hal_async::i2c::ErrorType for MockI2c<'_> {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal_async::i2c::I2c for MockI2c<'_> {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            match operations {
                [Operation::Write(w), Operation::Read(r)] => {
                    assert_eq!(w.len(), 1, "write_read prelude must be a single reg byte");
                    let mut responses = self.harness.read_responses.borrow_mut();
                    assert!(!responses.is_empty(), "no canned read response queued");
                    let response = responses.remove(0);
                    drop(responses);
                    assert_eq!(
                        response.len(),
                        r.len(),
                        "canned response length mismatch (queued {} vs requested {})",
                        response.len(),
                        r.len()
                    );
                    r.copy_from_slice(&response);
                    self.harness
                        .events
                        .borrow_mut()
                        .push(Event::WriteRead(address, w[0], r.len()));
                }
                other => panic!("unexpected operation shape: {} ops", other.len()),
            }
            Ok(())
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(future);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn read_vendor_id_issues_single_byte_read_at_0xa8() {
        let harness = Harness::new();
        harness.queue_read(vec![VENDOR_ID_FOCALTECH]);
        let mut bus = MockI2c { harness: &harness };
        let mut touch = Ft6336u::new(&mut bus);
        let id = block_on(touch.read_vendor_id()).unwrap();
        assert_eq!(id, VENDOR_ID_FOCALTECH);
        assert_eq!(
            harness.events(),
            vec![Event::WriteRead(CORES3_ADDRESS, REG_VENDOR_ID, 1)]
        );
    }

    #[test]
    fn read_touch_does_one_seven_byte_bulk_read_from_g_mode() {
        // Pins the wire format: a single atomic 7-byte read starting
        // at REG_G_MODE (0x00). A future "read TD_STATUS, then read
        // P1 block" refactor would split this into two transactions
        // and surface here, because non-atomic reads risk a touch
        // event landing between the two and reporting stale coords
        // against a fresh count.
        let harness = Harness::new();
        let mut response = vec![0u8; TOUCH_READ_LEN];
        response[OFFSET_TD_STATUS] = 0x01;
        response[OFFSET_P1_XH] = 0x01;
        response[OFFSET_P1_XL] = 0x00;
        response[OFFSET_P1_YH] = 0x00;
        response[OFFSET_P1_YL] = 0x80;
        harness.queue_read(response);
        let mut bus = MockI2c { harness: &harness };
        let mut touch = Ft6336u::new(&mut bus);
        let report = block_on(touch.read_touch()).unwrap();
        assert_eq!(report.fingers, 1);
        assert_eq!(report.first, Some((0x0100, 0x0080)));
        assert_eq!(
            harness.events(),
            vec![Event::WriteRead(CORES3_ADDRESS, REG_G_MODE, TOUCH_READ_LEN)]
        );
    }

    #[test]
    fn into_inner_returns_underlying_bus() {
        // into_inner consumes the driver and yields the bus back —
        // useful for single-task firmware that wants to reuse the bus
        // for another peripheral after touch init is done.
        let harness = Harness::new();
        let bus = MockI2c { harness: &harness };
        let touch = Ft6336u::new(bus);
        let _bus_back: MockI2c<'_> = touch.into_inner();
        // Pure type-level check — compiles iff the inferred type
        // matches the original bus.
    }

    #[test]
    fn error_from_blanket_wraps_in_i2c_variant() {
        // The `impl From<E> for Error<E>` blanket — every `?` operator
        // routes the bus error through this. Mock uses Infallible so
        // the runtime path can't fire; exercise the impl directly.
        let err: Error<&'static str> = "bus go boom".into();
        match err {
            Error::I2c(s) => assert_eq!(s, "bus go boom"),
        }
    }
}
