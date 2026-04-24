//! # scservo
//!
//! `no_std` driver for the Feetech `SCServo` (`SCSCL` / `SCS0009`) family of
//! smart serial servos. The servos share a half-duplex TTL bus at 1 Mbaud
//! (default), each addressed by a 1-byte ID. This crate speaks the
//! Feetech packet protocol over any [`embedded_io_async::Write`] (for
//! position / torque commands) and optionally [`embedded_io_async::Read`]
//! (for [`Scservo::ping`], the boot-time health check).
//!
//! ## Timeouts
//!
//! Neither write nor [`Scservo::ping`] implement timeouts themselves —
//! UART reads block until the full response arrives, so a disconnected
//! or unpowered servo hangs forever. Callers are expected to wrap
//! `ping(id)` with their runtime's timeout primitive (e.g.
//! `embassy_time::with_timeout(Duration::from_millis(10), bus.ping(1))`).
//! 10 ms is generous — a round-trip at 1 Mbaud is <200 µs.
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

use embedded_io_async::{Read, ReadExactError, Write};

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

/// SCSCL memory-table address of the present-position low byte. A
/// 2-byte big-endian read here yields the current servo position in
/// the same 0..=1023 count space as [`ADDR_GOAL_POSITION`].
pub const ADDR_PRESENT_POSITION: u8 = 56;

/// SCSCL memory-table address of the present-voltage byte. Units: 0.1 V
/// per count (e.g. 74 = 7.4 V).
pub const ADDR_PRESENT_VOLTAGE: u8 = 62;

/// SCSCL memory-table address of the present-temperature byte. Units:
/// °C (direct, no scaling).
pub const ADDR_PRESENT_TEMPERATURE: u8 = 63;

/// SCSCL memory-table address of the moving-flag byte. 0 = settled,
/// non-zero = actively tracking toward goal position.
pub const ADDR_MOVING: u8 = 66;

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
/// Maximum valid servo position count. SCSCL / SCS0009 use a 0..=1023
/// range; values above are rejected by [`Scservo::write_position`] with
/// [`Error::PositionOutOfRange`].
pub const POSITION_MAX: u16 = 1023;
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
    /// v1 write surface (`WritePos`, `WriteTorque`) never reaches
    /// this; the arm exists so arbitrary-length writers added later
    /// stay bounded.
    PayloadTooLarge,
    /// `read_exact` returned `UnexpectedEof` — the UART closed before
    /// a full response arrived. On open-ended serial links this rarely
    /// fires in practice; a hung slave produces a timeout (caller's
    /// responsibility) rather than EOF.
    NoResponse,
    /// Response packet didn't parse: wrong header bytes or the
    /// responding ID doesn't match what we asked.
    MalformedResponse,
    /// Response packet arrived but its checksum didn't verify.
    ChecksumMismatch,
    /// Caller passed a position value above [`POSITION_MAX`] (1023) to
    /// [`Scservo::write_position`]. The servo would interpret the high
    /// bits as a different address in its memory table, so we reject at
    /// the driver boundary.
    PositionOutOfRange(u16),
}

impl<E> From<ReadExactError<E>> for Error<E> {
    fn from(e: ReadExactError<E>) -> Self {
        match e {
            ReadExactError::UnexpectedEof => Self::NoResponse,
            ReadExactError::Other(e) => Self::Uart(e),
        }
    }
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

    /// Command servo `id` to `position` (step count 0..=[`POSITION_MAX`]),
    /// transitioning over `time_ms` milliseconds at `speed` (0 = use
    /// time control).
    ///
    /// See [`POSITION_CENTER`] / [`POSITION_PER_DEGREE`] for angle
    /// conversion. Positions above [`POSITION_MAX`] are rejected with
    /// [`Error::PositionOutOfRange`] rather than forwarded verbatim:
    /// the servo interprets the high bits as a different memory-table
    /// address, so silently sending them out-of-range is strictly
    /// worse than returning an error. For tighter per-application
    /// limits, write `MIN_ANGLE_LIMIT` / `MAX_ANGLE_LIMIT` directly.
    ///
    /// # Errors
    /// - [`Error::PositionOutOfRange`] if `position > POSITION_MAX`.
    /// - [`Error::Uart`] if the transport fails.
    pub async fn write_position(
        &mut self,
        id: u8,
        position: u16,
        time_ms: u16,
        speed: u16,
    ) -> Result<(), Error<W::Error>> {
        if position > POSITION_MAX {
            return Err(Error::PositionOutOfRange(position));
        }
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

impl<U: Read + Write> Scservo<U> {
    /// Probe servo `id` with a [`Instruction::Ping`] and wait for the
    /// 6-byte status response. Returns `Ok(())` iff a well-formed
    /// response with matching ID and valid checksum arrives.
    ///
    /// Does **not** enforce a timeout internally — a hung or absent
    /// slave leaves this future pending forever. Callers wrap with
    /// their runtime's timeout primitive, e.g.
    ///
    /// ```no_run
    /// # use embedded_io_async::{Read, Write};
    /// # async fn demo<U: Read + Write>(uart: U) {
    /// # let mut bus = scservo::Scservo::new(uart);
    /// # let timeout_fut = core::future::pending::<()>();
    /// // Pseudocode — use embassy_time::with_timeout or equivalent.
    /// let _ = bus.ping(1).await;
    /// # }
    /// ```
    ///
    /// Echo-on-bus note: if the physical half-duplex converter echoes
    /// outgoing TX back into RX, this read may see the 6-byte outgoing
    /// PING packet before the 6-byte response. On CoreS3 with the
    /// standard Stack-chan base this has not been observed; if it
    /// becomes a problem, switch to `Instruction::Read` (different
    /// packet size, distinguishable from its own echo).
    ///
    /// # Errors
    /// - [`Error::Uart`] on transport failure.
    /// - [`Error::NoResponse`] if the UART closes before a full
    ///   response arrives.
    /// - [`Error::MalformedResponse`] if the header or responding ID
    ///   don't match expectations.
    /// - [`Error::ChecksumMismatch`] if the response checksum is
    ///   invalid.
    pub async fn ping(&mut self, id: u8) -> Result<(), Error<U::Error>> {
        // PING outbound: FF FF ID 02 01 ~(ID+2+1).
        let checksum = !id.wrapping_add(0x02).wrapping_add(Instruction::Ping as u8);
        let outbound = [
            HEADER_BYTE,
            HEADER_BYTE,
            id,
            0x02,
            Instruction::Ping as u8,
            checksum,
        ];
        self.uart.write_all(&outbound).await?;

        // Expected response layout: FF FF ID 02 Error ~checksum (6 bytes).
        let mut response = [0u8; 6];
        self.uart.read_exact(&mut response).await?;

        if response[0] != HEADER_BYTE || response[1] != HEADER_BYTE {
            return Err(Error::MalformedResponse);
        }
        if response[2] != id {
            return Err(Error::MalformedResponse);
        }
        // msgLen for a PING response is 2 (error + checksum).
        if response[3] != 0x02 {
            return Err(Error::MalformedResponse);
        }
        // Checksum covers ID..=error byte.
        let sum = response[2]
            .wrapping_add(response[3])
            .wrapping_add(response[4]);
        if response[5] != !sum {
            return Err(Error::ChecksumMismatch);
        }
        // response[4] is the servo's error byte — non-zero signals
        // voltage / overload / angle-limit faults, but the servo is
        // still "present". PING cares about presence, not fault-free;
        // return Ok. Callers that need the error byte can use a
        // dedicated status read later.
        Ok(())
    }

    /// Low-level [`Instruction::Read`] that fills `buf` with `buf.len()`
    /// bytes read starting at `mem_addr` on servo `id`.
    ///
    /// Outbound packet layout:
    ///
    /// ```text
    /// | 0xFF | 0xFF | ID | 0x04 | 0x02 | MemAddr | Len | ~Checksum |
    /// ```
    ///
    /// Response layout:
    ///
    /// ```text
    /// | 0xFF | 0xFF | ID | Len+2 | Error | Data[0..Len] | ~Checksum |
    /// ```
    ///
    /// Same timeout responsibility as [`Scservo::ping`] — wrap with
    /// `embassy_time::with_timeout` for bounded waits.
    ///
    /// # Errors
    /// - [`Error::PayloadTooLarge`] if `buf.len() > MAX_DATA_BYTES`.
    /// - [`Error::Uart`] on transport failure.
    /// - [`Error::NoResponse`] / [`Error::MalformedResponse`] /
    ///   [`Error::ChecksumMismatch`] on bad responses.
    pub async fn read_memory(
        &mut self,
        id: u8,
        mem_addr: u8,
        buf: &mut [u8],
    ) -> Result<(), Error<U::Error>> {
        if buf.len() > MAX_DATA_BYTES {
            return Err(Error::PayloadTooLarge);
        }
        // data.len() is ≤ MAX_DATA_BYTES (6), so fits u8 trivially.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "len() ≤ MAX_DATA_BYTES (6), fits u8"
        )]
        let read_len = buf.len() as u8;

        // Outbound: FF FF ID 04 02 ADDR LEN ~checksum (8 bytes).
        // Checksum = ~(ID + 4 + Read + mem_addr + len).
        let sum = id
            .wrapping_add(0x04)
            .wrapping_add(Instruction::Read as u8)
            .wrapping_add(mem_addr)
            .wrapping_add(read_len);
        let outbound = [
            HEADER_BYTE,
            HEADER_BYTE,
            id,
            0x04,
            Instruction::Read as u8,
            mem_addr,
            read_len,
            !sum,
        ];
        self.uart.write_all(&outbound).await?;

        // Response: 2 header + 1 id + 1 len-field + 1 error + N data + 1 checksum.
        let response_total = 6 + buf.len();
        let mut response = [0u8; MAX_PACKET_BYTES];
        self.uart
            .read_exact(&mut response[..response_total])
            .await?;

        if response[0] != HEADER_BYTE || response[1] != HEADER_BYTE {
            return Err(Error::MalformedResponse);
        }
        if response[2] != id {
            return Err(Error::MalformedResponse);
        }
        // msgLen for a READ response is `data_len + 2` (error + checksum).
        #[allow(
            clippy::cast_possible_truncation,
            reason = "buf.len() ≤ MAX_DATA_BYTES; the +2 stays well under u8::MAX"
        )]
        let expected_msg_len = (buf.len() + 2) as u8;
        if response[3] != expected_msg_len {
            return Err(Error::MalformedResponse);
        }

        // Checksum covers ID..=last data byte (excludes the checksum byte
        // itself).
        let mut check_sum: u8 = 0;
        for &b in &response[2..response_total - 1] {
            check_sum = check_sum.wrapping_add(b);
        }
        if response[response_total - 1] != !check_sum {
            return Err(Error::ChecksumMismatch);
        }

        // Data bytes start at index 5; skip the 2 header + 1 id + 1 msgLen +
        // 1 error bytes before them.
        buf.copy_from_slice(&response[5..5 + buf.len()]);
        Ok(())
    }

    /// Read the current position of servo `id` — the live encoder
    /// reading, in the same 0..=1023 count space as the goal position.
    /// Big-endian 2-byte field at [`ADDR_PRESENT_POSITION`].
    ///
    /// # Errors
    /// Same as [`Scservo::read_memory`].
    pub async fn read_position(&mut self, id: u8) -> Result<u16, Error<U::Error>> {
        let mut buf = [0u8; 2];
        self.read_memory(id, ADDR_PRESENT_POSITION, &mut buf)
            .await?;
        Ok(u16::from_be_bytes(buf))
    }

    /// Read the current supply voltage of servo `id`. Units: 0.1 V per
    /// count (e.g. 74 → 7.4 V).
    ///
    /// # Errors
    /// Same as [`Scservo::read_memory`].
    pub async fn read_voltage(&mut self, id: u8) -> Result<u8, Error<U::Error>> {
        let mut buf = [0u8; 1];
        self.read_memory(id, ADDR_PRESENT_VOLTAGE, &mut buf).await?;
        Ok(buf[0])
    }

    /// Read the current internal temperature of servo `id`, in °C.
    ///
    /// # Errors
    /// Same as [`Scservo::read_memory`].
    pub async fn read_temperature(&mut self, id: u8) -> Result<u8, Error<U::Error>> {
        let mut buf = [0u8; 1];
        self.read_memory(id, ADDR_PRESENT_TEMPERATURE, &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Read the moving flag of servo `id` — true while the servo is
    /// actively tracking toward its goal position, false once settled.
    /// Useful for "wait until move completes" loops during calibration.
    ///
    /// # Errors
    /// Same as [`Scservo::read_memory`].
    pub async fn read_moving(&mut self, id: u8) -> Result<bool, Error<U::Error>> {
        let mut buf = [0u8; 1];
        self.read_memory(id, ADDR_MOVING, &mut buf).await?;
        Ok(buf[0] != 0)
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
    /// byte-level packet assertions, and returns pre-loaded bytes on
    /// reads (for ping-response testing).
    struct MockUart {
        /// Bytes the driver has written to us.
        written: RefCell<Vec<u8>>,
        /// Bytes we'll hand back, in order, to driver reads. Tests
        /// pre-load this with the response packet they expect the
        /// driver to consume.
        rx_queue: RefCell<Vec<u8>>,
    }

    impl MockUart {
        fn new() -> Self {
            Self {
                written: RefCell::new(Vec::new()),
                rx_queue: RefCell::new(Vec::new()),
            }
        }

        fn queue_rx(&self, bytes: &[u8]) {
            self.rx_queue.borrow_mut().extend_from_slice(bytes);
        }
    }

    impl embedded_io_async::ErrorType for MockUart {
        type Error = core::convert::Infallible;
    }

    impl embedded_io_async::Write for MockUart {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.written.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl embedded_io_async::Read for MockUart {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            // Pop up to buf.len() bytes from the head of rx_queue.
            let mut q = self.rx_queue.borrow_mut();
            let take = buf.len().min(q.len());
            for (dst, src) in buf.iter_mut().zip(q.drain(..take)) {
                *dst = src;
            }
            Ok(take)
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
        assert_eq!(bus.uart.written.borrow().as_slice(), expected);
    }

    #[test]
    fn write_position_broadcast_id_uses_0xfe() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_position(BROADCAST_ID, 0, 0, 0)).unwrap();
        assert_eq!(bus.uart.written.borrow()[2], 0xFE);
    }

    #[test]
    fn write_torque_enable_packs_single_byte() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_torque_enable(1, true)).unwrap();
        // FF FF 01 04 03 28 01 CE
        let expected: &[u8] = &[0xFF, 0xFF, 0x01, 0x04, 0x03, 0x28, 0x01, 0xCE];
        assert_eq!(bus.uart.written.borrow().as_slice(), expected);
    }

    #[test]
    fn checksum_is_bitwise_not_of_field_sum() {
        let mut bus = Scservo::new(MockUart::new());
        block_on(bus.write_position(2, 700, 30, 100)).unwrap();
        let buf = bus.uart.written.borrow();
        let expected_sum = buf[2..12].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        assert_eq!(buf[12], !expected_sum);
    }

    #[test]
    fn write_position_accepts_exact_max_position() {
        let mut bus = Scservo::new(MockUart::new());
        // POSITION_MAX (1023) is inside the valid range — must succeed.
        block_on(bus.write_position(1, POSITION_MAX, 0, 0)).unwrap();
        assert!(!bus.uart.written.borrow().is_empty());
    }

    #[test]
    fn write_position_rejects_above_max() {
        let mut bus = Scservo::new(MockUart::new());
        let err = block_on(bus.write_position(1, POSITION_MAX + 1, 0, 0));
        assert!(matches!(err, Err(Error::PositionOutOfRange(p)) if p == POSITION_MAX + 1));
        // No packet was transmitted — the guard runs before serialisation.
        assert!(bus.uart.written.borrow().is_empty());
    }

    #[test]
    fn payload_too_large_is_typed_error() {
        let mut bus = Scservo::new(MockUart::new());
        let oversize = [0u8; MAX_DATA_BYTES + 1];
        let err = block_on(bus.write_memory(1, 0, &oversize));
        assert!(matches!(err, Err(Error::PayloadTooLarge)));
    }

    #[test]
    fn ping_sends_correct_outbound_packet() {
        let mut bus = Scservo::new(MockUart::new());
        // Pre-queue a valid response so the read doesn't hang. We
        // don't assert on its value here; this test checks the outbound.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x01, 0x02, 0x00, 0xFC]);
        block_on(bus.ping(1)).unwrap();
        // Expected outbound: FF FF 01 02 01 ~(1+2+1) = ~4 = FB.
        let expected: &[u8] = &[0xFF, 0xFF, 0x01, 0x02, 0x01, 0xFB];
        assert_eq!(bus.uart.written.borrow().as_slice(), expected);
    }

    #[test]
    fn ping_succeeds_on_valid_response() {
        let mut bus = Scservo::new(MockUart::new());
        // Response: FF FF 01 02 Error=0 ~(1+2+0) = ~3 = FC.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x01, 0x02, 0x00, 0xFC]);
        assert!(block_on(bus.ping(1)).is_ok());
    }

    #[test]
    fn ping_succeeds_when_error_byte_nonzero() {
        // Non-zero error byte signals fault (overload, voltage, etc.),
        // but the servo is still present — ping returns Ok.
        let mut bus = Scservo::new(MockUart::new());
        let err_byte = 0x20;
        let checksum = !(1u8.wrapping_add(2).wrapping_add(err_byte));
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x02, err_byte, checksum]);
        assert!(block_on(bus.ping(1)).is_ok());
    }

    #[test]
    fn ping_rejects_bad_header() {
        let mut bus = Scservo::new(MockUart::new());
        // Header byte 0 corrupted.
        bus.uart.queue_rx(&[0x00, 0xFF, 0x01, 0x02, 0x00, 0xFC]);
        let err = block_on(bus.ping(1));
        assert!(matches!(err, Err(Error::MalformedResponse)));
    }

    #[test]
    fn ping_rejects_wrong_id_in_response() {
        let mut bus = Scservo::new(MockUart::new());
        // Response from ID=2 when we asked for ID=1.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x02, 0x02, 0x00, 0xFB]);
        let err = block_on(bus.ping(1));
        assert!(matches!(err, Err(Error::MalformedResponse)));
    }

    #[test]
    fn ping_rejects_bad_checksum() {
        let mut bus = Scservo::new(MockUart::new());
        // Valid structure but checksum byte corrupted.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x01, 0x02, 0x00, 0xFF]);
        let err = block_on(bus.ping(1));
        assert!(matches!(err, Err(Error::ChecksumMismatch)));
    }

    #[test]
    fn ping_no_response_maps_to_typed_error() {
        // Empty rx queue -> read_exact returns UnexpectedEof (the mock
        // returns 0 bytes on read, which read_exact treats as EOF).
        let mut bus = Scservo::new(MockUart::new());
        let err = block_on(bus.ping(1));
        assert!(matches!(err, Err(Error::NoResponse)));
    }

    // ----- READ instruction ------------------------------------------

    #[test]
    fn read_position_sends_correct_outbound_packet() {
        let mut bus = Scservo::new(MockUart::new());
        // Pre-queue a valid 2-byte position response so read_exact doesn't hang.
        // Response: FF FF 01 04 00 02 00 checksum. sum = 1+4+0+2+0 = 7, !7 = 0xF8.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x04, 0x00, 0x02, 0x00, 0xF8]);
        let _ = block_on(bus.read_position(1));
        // Expected outbound: FF FF 01 04 02 38 02 ~(1+4+2+56+2) = ~65 = 0xBE.
        let expected: &[u8] = &[0xFF, 0xFF, 0x01, 0x04, 0x02, 0x38, 0x02, 0xBE];
        assert_eq!(bus.uart.written.borrow().as_slice(), expected);
    }

    #[test]
    fn read_position_decodes_big_endian_response() {
        let mut bus = Scservo::new(MockUart::new());
        // Position = 0x0200 = 512 (center). Data bytes: [0x02, 0x00].
        // Full response: FF FF 01 04 00 02 00 checksum.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x04, 0x00, 0x02, 0x00, 0xF8]);
        let pos = block_on(bus.read_position(1)).unwrap();
        assert_eq!(pos, 512);
    }

    #[test]
    fn read_voltage_decodes_single_byte() {
        let mut bus = Scservo::new(MockUart::new());
        // Voltage = 74 (= 7.4 V). msgLen = 3. sum = 1+3+0+74 = 78, !78 = 0xB1.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x01, 0x03, 0x00, 74, 0xB1]);
        let v = block_on(bus.read_voltage(1)).unwrap();
        assert_eq!(v, 74);
    }

    #[test]
    fn read_temperature_decodes_single_byte() {
        let mut bus = Scservo::new(MockUart::new());
        // Temp = 32°C. sum = 1+3+0+32 = 36, !36 = 0xDB.
        bus.uart.queue_rx(&[0xFF, 0xFF, 0x01, 0x03, 0x00, 32, 0xDB]);
        let t = block_on(bus.read_temperature(1)).unwrap();
        assert_eq!(t, 32);
    }

    #[test]
    fn read_moving_decodes_bool() {
        let mut bus = Scservo::new(MockUart::new());
        // Moving = 1 (true). sum = 1+3+0+1 = 5, !5 = 0xFA.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x03, 0x00, 0x01, 0xFA]);
        let moving = block_on(bus.read_moving(1)).unwrap();
        assert!(moving);
    }

    #[test]
    fn read_moving_false_on_zero_byte() {
        let mut bus = Scservo::new(MockUart::new());
        // Moving = 0 (false). sum = 1+3+0+0 = 4, !4 = 0xFB.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x03, 0x00, 0x00, 0xFB]);
        let moving = block_on(bus.read_moving(1)).unwrap();
        assert!(!moving);
    }

    #[test]
    fn read_memory_rejects_oversized_buf() {
        let mut bus = Scservo::new(MockUart::new());
        let mut buf = [0u8; MAX_DATA_BYTES + 1];
        let err = block_on(bus.read_memory(1, 0, &mut buf));
        assert!(matches!(err, Err(Error::PayloadTooLarge)));
    }

    #[test]
    fn read_memory_rejects_wrong_length_field() {
        let mut bus = Scservo::new(MockUart::new());
        // Response claims msgLen = 5 but we asked for 2 data bytes → expect 4.
        // Even with a matching checksum the header check fails first.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x05, 0x00, 0x02, 0x00, 0xF7]);
        let err = block_on(bus.read_position(1));
        assert!(matches!(err, Err(Error::MalformedResponse)));
    }

    #[test]
    fn read_memory_rejects_bad_checksum() {
        let mut bus = Scservo::new(MockUart::new());
        // Valid structure but checksum byte corrupted.
        bus.uart
            .queue_rx(&[0xFF, 0xFF, 0x01, 0x04, 0x00, 0x02, 0x00, 0x00]);
        let err = block_on(bus.read_position(1));
        assert!(matches!(err, Err(Error::ChecksumMismatch)));
    }
}
