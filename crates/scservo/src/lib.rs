//! # scservo
//!
//! `no_std` driver for the Feetech `SCServo` (`SCSCL` / `SCS0009`) family of
//! smart serial servos. The servos share a half-duplex TTL bus at 1 Mbaud
//! (default), each addressed by a 1-byte ID. This crate speaks the
//! Feetech packet protocol over any [`embedded_io_async::Write`] — for
//! v1, writes only (position and torque commands); reads and feedback
//! may arrive in a later release.
//!
//! ## Packet format
//!
//! ```text
//! | 0xFF | 0xFF | ID | msgLen | Instruction | MemAddr | Data... | ~Checksum |
//! ```
//!
//! - `msgLen` = payload length after the `msgLen` byte itself. For a
//!   [`Instruction::Write`] of `N` data bytes to a register,
//!   `msgLen = N + 3` (instruction + `mem_addr` + checksum). For
//!   data-less instructions (`PING`), `msgLen = 2` and the `MemAddr`
//!   byte is omitted.
//! - `Checksum` = `~(ID + msgLen + Instruction + MemAddr + sum(Data))`.
//! - 16-bit values are **big-endian** (SCSCL `End = 1`).
//!
//! ## Position counts
//!
//! SCS0009 and siblings use a 0..=1023 step range across ~300° of travel,
//! centered at 512. [`POSITION_CENTER`] and [`POSITION_PER_DEGREE`] are
//! provided so callers can convert angle → step without repeating the
//! math. At the typical ±45° commanded from this project, positions
//! stay comfortably inside the valid range.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_io_async::Write;
//! # async fn demo<U: Write>(uart: U) -> Result<(), scservo::Error<U::Error>> {
//! let mut bus = scservo::Scservo::new(uart);
//! // Command servo ID=1 to center (512), travel over 20 ms, speed 0.
//! bus.write_position(1, 512, 20, 0).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_io_async::Write;

/// Broadcast ID — all servos on the bus act on the packet, no response.
pub const BROADCAST_ID: u8 = 0xFE;

/// Byte value of the protocol header (sent twice at the start of every
/// packet).
const HEADER_BYTE: u8 = 0xFF;

/// SCSCL memory-table address of the goal-position low byte.
///
/// The [`Instruction::Write`] for a pose lands 6 bytes at this address:
/// position(2), time(2), speed(2), big-endian.
pub const ADDR_GOAL_POSITION: u8 = 42;

/// SCSCL memory-table address of the torque-enable byte.
pub const ADDR_TORQUE_ENABLE: u8 = 40;

/// Protocol instruction codes (from `INST.h` in the Feetech reference
/// library).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// Query — servo responds with its status byte.
    Ping = 0x01,
    /// Read `N` bytes from a register.
    Read = 0x02,
    /// Write `N` bytes to a register.
    Write = 0x03,
    /// Register a deferred write (committed by
    /// [`Instruction::RegAction`]).
    RegWrite = 0x04,
    /// Execute all pending [`Instruction::RegWrite`]s.
    RegAction = 0x05,
    /// Broadcast write of identical-shape payload to multiple IDs.
    SyncWrite = 0x83,
}

/// Servo position count at the mechanical center (neutral).
pub const POSITION_CENTER: u16 = 512;
/// Position counts per degree for SCSCL / SCS0009: 1023 counts across
/// 300° of travel.
pub const POSITION_PER_DEGREE: f32 = 1023.0 / 300.0;

/// Maximum data payload for [`Scservo::write_memory`]. Sized to cover
/// every instruction the crate emits (`WritePos` uses 6); the bound is
/// enforced at the entry with a typed [`Error::PayloadTooLarge`].
pub const MAX_DATA_BYTES: usize = 6;

/// Total packet bytes = 2 header + 1 id + 1 msgLen + 1 instr + 1 addr
///                    + `MAX_DATA_BYTES` + 1 checksum.
const MAX_PACKET_BYTES: usize = 7 + MAX_DATA_BYTES;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying UART.
    Uart(E),
    /// Caller passed more than [`MAX_DATA_BYTES`] bytes of data. The
    /// v1 surface (`WritePos`, `WriteTorque`) never reaches this; the arm
    /// exists so arbitrary-length writers added later stay bounded.
    PayloadTooLarge,
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::Uart(e)
    }
}

/// `SCServo` bus driver. Owns the UART writer; construct with
/// [`Scservo::new`] and call the instruction methods.
#[derive(Debug)]
pub struct Scservo<W> {
    /// Underlying half-duplex UART transmitter.
    uart: W,
}

impl<W: Write> Scservo<W> {
    /// Wrap a UART writer. The caller is responsible for configuring
    /// the baud rate (typically 1 Mbaud for Feetech SCS series).
    #[must_use]
    pub const fn new(uart: W) -> Self {
        Self { uart }
    }

    /// Release the UART writer back to the caller.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.uart
    }

    /// Command servo `id` to `position` (step count 0..=1023),
    /// transitioning over `time_ms` milliseconds at `speed` (0 = use
    /// time control).
    ///
    /// See [`POSITION_CENTER`] / [`POSITION_PER_DEGREE`] for angle
    /// conversion. Values outside 0..=1023 are clamped implicitly by
    /// the servo's angle-limit registers, which is the preferred layer
    /// for enforcing safety — ship custom limits by writing the
    /// `MIN_ANGLE_LIMIT` / `MAX_ANGLE_LIMIT` registers directly.
    ///
    /// # Errors
    /// Returns the UART transport error if the write fails.
    pub async fn write_position(
        &mut self,
        id: u8,
        position: u16,
        time_ms: u16,
        speed: u16,
    ) -> Result<(), Error<W::Error>> {
        let data = [
            (position >> 8) as u8,
            (position & 0xFF) as u8,
            (time_ms >> 8) as u8,
            (time_ms & 0xFF) as u8,
            (speed >> 8) as u8,
            (speed & 0xFF) as u8,
        ];
        self.write_memory(id, ADDR_GOAL_POSITION, &data).await
    }

    /// Enable or disable holding torque on servo `id`. Servos default
    /// to torque-on; only call this explicitly if you want to
    /// free-wheel the shaft (e.g. hand-positioning during calibration).
    ///
    /// # Errors
    /// Returns the UART transport error if the write fails.
    pub async fn write_torque_enable(
        &mut self,
        id: u8,
        enabled: bool,
    ) -> Result<(), Error<W::Error>> {
        let byte = u8::from(enabled);
        self.write_memory(id, ADDR_TORQUE_ENABLE, &[byte]).await
    }

    /// Low-level: [`Instruction::Write`] targeting `mem_addr` with
    /// `data`. All higher-level writers ultimately dispatch through
    /// here.
    ///
    /// # Errors
    /// - [`Error::PayloadTooLarge`] if `data.len() > MAX_DATA_BYTES`.
    /// - [`Error::Uart`] if the transport fails.
    pub async fn write_memory(
        &mut self,
        id: u8,
        mem_addr: u8,
        data: &[u8],
    ) -> Result<(), Error<W::Error>> {
        if data.len() > MAX_DATA_BYTES {
            return Err(Error::PayloadTooLarge);
        }

        let mut packet = [0u8; MAX_PACKET_BYTES];
        // msgLen = data.len() + 3 (instruction + mem_addr + checksum).
        // data.len() is ≤ MAX_DATA_BYTES (6), so msgLen ≤ 9 — fits in u8.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "data.len() ≤ MAX_DATA_BYTES (6), so msgLen ≤ 9 — fits u8"
        )]
        let msg_len = (data.len() + 3) as u8;

        packet[0] = HEADER_BYTE;
        packet[1] = HEADER_BYTE;
        packet[2] = id;
        packet[3] = msg_len;
        packet[4] = Instruction::Write as u8;
        packet[5] = mem_addr;
        packet[6..6 + data.len()].copy_from_slice(data);

        // Checksum covers ID..=last data byte.
        let mut sum: u8 = 0;
        for &b in &packet[2..6 + data.len()] {
            sum = sum.wrapping_add(b);
        }
        packet[6 + data.len()] = !sum;

        let total = 7 + data.len();
        self.uart.write_all(&packet[..total]).await?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "MockUart's Infallible error type makes unwrap() on writes \
              sound — and clearer than matches! for test readability"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;

    /// Host-side UART mock that records every write into a Vec for
    /// byte-level packet assertions.
    struct MockUart {
        buf: RefCell<Vec<u8>>,
    }

    impl MockUart {
        fn new() -> Self {
            Self {
                buf: RefCell::new(Vec::new()),
            }
        }
    }

    impl embedded_io_async::ErrorType for MockUart {
        type Error = core::convert::Infallible;
    }

    impl embedded_io_async::Write for MockUart {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.buf.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// Tiny zero-dep future poller — this crate has no async executor
    /// dep. `Write` futures in `MockUart` are always `Ready`, so a
    /// single poll returns the value.
    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};
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
    fn write_position_matches_feetech_reference_bytes() {
        let mut bus = Scservo::new(MockUart::new());
        // WritePos(id=1, pos=512, time=20, speed=0):
        //   FF FF 01 09 03 2A 02 00 00 14 00 00 B2
        //   checksum = ~(1+9+3+42+2+0+0+20+0+0) = ~77 = 0xB2
        block_on(bus.write_position(1, 512, 20, 0)).unwrap();
        let expected: &[u8] = &[
            0xFF, 0xFF, 0x01, 0x09, 0x03, 0x2A, 0x02, 0x00, 0x00, 0x14, 0x00, 0x00, 0xB2,
        ];
        assert_eq!(bus.uart.buf.borrow().as_slice(), expected);
    }

    #[test]
    fn write_position_broadcast_id_uses_0xfe() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_position(BROADCAST_ID, 0, 0, 0)).unwrap();
        assert_eq!(bus.uart.buf.borrow()[2], 0xFE);
    }

    #[test]
    fn write_torque_enable_packs_single_byte() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_torque_enable(1, true)).unwrap();
        // FF FF 01 04 03 28 01 CE
        let expected: &[u8] = &[0xFF, 0xFF, 0x01, 0x04, 0x03, 0x28, 0x01, 0xCE];
        assert_eq!(bus.uart.buf.borrow().as_slice(), expected);
    }

    #[test]
    fn checksum_is_bitwise_not_of_field_sum() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_position(2, 700, 30, 100)).unwrap();
        let buf = bus.uart.buf.borrow();
        let expected_sum = buf[2..12].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        assert_eq!(buf[12], !expected_sum);
    }

    #[test]
    fn payload_too_large_is_typed_error() {
        let mut bus = Scservo::new(MockUart::new());
        let oversize = [0u8; MAX_DATA_BYTES + 1];
        let err = block_on(bus.write_memory(1, 0, &oversize));
        assert!(matches!(err, Err(Error::PayloadTooLarge)));
    }
}
