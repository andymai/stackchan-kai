//! # gc0308
//!
//! `no_std` async I²C control-path driver for the `GalaxyCore` GC0308
//! VGA CMOS image sensor. Targets the M5Stack CoreS3 board, where
//! the GC0308 is the on-camera FPC behind the LTR-553 ribbon.
//!
//! Covers SCCB-style (I²C-compatible) register access for resolution,
//! output format, mirror / flip, and streaming gating. The actual pixel
//! transport happens over a parallel DVP bus driven by the MCU's
//! `LCD_CAM` peripheral; that lives in the firmware crate.
//!
//! ## CoreS3 wiring quirks
//!
//! On the M5Stack CoreS3, the camera's PWDN, RESET, and XCLK pins are
//! not connected to ESP32 GPIOs. The sensor self-clocks from an
//! on-board oscillator, soft-reset is via SCCB only, and "power down"
//! is implemented by gating the data-output enable bit ([`REG_IO_OUTPUT`])
//! through [`Gc0308::set_streaming`] instead of toggling PWDN.
//!
//! ## Example
//!
//! ```no_run
//! # use embedded_hal_async::{i2c::I2c, delay::DelayNs};
//! # async fn demo<B, D>(bus: B, mut delay: D) -> Result<(), gc0308::Error<B::Error>>
//! # where B: I2c, D: DelayNs {
//! let mut cam = gc0308::Gc0308::new(bus);
//! cam.init(&mut delay).await?;
//! cam.set_format(gc0308::Format::Rgb565).await?;
//! cam.set_framesize_qvga().await?;
//! cam.set_streaming(true).await?;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use embedded_hal_async::{delay::DelayNs, i2c::I2c};

mod regs;

pub use regs::{DEFAULT_REGS, REG_PAGE_SELECT};

/// 7-bit I²C address. GC0308 has a fixed address on the SCCB bus.
pub const ADDRESS: u8 = 0x21;

/// Expected value of the [`REG_CHIP_ID`] register on a genuine GC0308.
pub const CHIP_ID: u8 = 0x9B;

/// `CHIP_ID` (PID) register. Read-only. Expected to return [`CHIP_ID`].
///
/// Confirmed against the live CoreS3 sensor at I²C address `0x21` —
/// register `0x00` returns `0x9B`. The Linux kernel `gc0308` driver
/// (`drivers/media/i2c/gc0308.c`, `GC0308_REG_VAL_PID`) agrees.
/// Earlier revisions of this crate read `0xF0`; that's the
/// "frame-rate / gain trim" register from the upstream
/// `gc0308_settings.h` table, not the PID, and reads back `0xF3` on
/// real hardware.
pub const REG_CHIP_ID: u8 = 0x00;

/// `RESET_RELATED` register. Writing `SOFT_RESET_PAYLOAD` issues a
/// software reset (clears registers + holds in reset until the table
/// is reprogrammed).
pub const REG_RESET: u8 = 0xFE;

/// `OUTPUT_FMT` register on page 0. The low nibble selects the output
/// format (see [`Format`]).
pub const REG_OUTPUT_FMT: u8 = 0x24;

/// `CISCTL_MODE1` on page 0. Bit 0 = horizontal mirror, bit 1 = vertical
/// flip. Default register value (`0x10`) leaves both off.
pub const REG_CISCTL_MODE1: u8 = 0x14;

/// `IO_OUTPUT_EN` on page 0.
///
/// The low two bits gate PCLK and the data bus. Setting `0x00` parks
/// the DVP outputs (effective "stream off" without a PWDN pin); `0x02`
/// matches the default-register table and enables the parallel output.
pub const REG_IO_OUTPUT: u8 = 0xF2;

/// Page-0 row-start high byte (windowing).
const REG_ROW_START_H: u8 = 0x05;
/// Page-0 row-start low byte.
const REG_ROW_START_L: u8 = 0x06;
/// Page-0 column-start high byte.
const REG_COL_START_H: u8 = 0x07;
/// Page-0 column-start low byte.
const REG_COL_START_L: u8 = 0x08;
/// Page-0 window-height high byte.
const REG_WIN_HEIGHT_H: u8 = 0x09;
/// Page-0 window-height low byte.
const REG_WIN_HEIGHT_L: u8 = 0x0A;
/// Page-0 window-width high byte.
const REG_WIN_WIDTH_H: u8 = 0x0B;
/// Page-0 window-width low byte.
const REG_WIN_WIDTH_L: u8 = 0x0C;

/// Page-1 subsample-enable register (bit 7 = enable).
const REG_SUBSAMPLE_EN: u8 = 0x53;
/// Page-1 subsample-mode register (per-axis ratio nibbles).
const REG_SUBSAMPLE_MODE: u8 = 0x54;
/// Page-1 secondary subsample-enable (bit 0 = enable).
const REG_SUBSAMPLE_EN2: u8 = 0x55;
/// Page-1 luma row-pattern register 0.
const REG_SUBSAMPLE_Y0: u8 = 0x56;
/// Page-1 luma row-pattern register 1.
const REG_SUBSAMPLE_Y1: u8 = 0x57;
/// Page-1 chroma row-pattern register 0.
const REG_SUBSAMPLE_UV0: u8 = 0x58;
/// Page-1 chroma row-pattern register 1.
const REG_SUBSAMPLE_UV1: u8 = 0x59;

/// Streaming-on value for [`REG_IO_OUTPUT`]. Mirrors the default-register
/// table entry so the chip lands in the same state after a stream-cycle.
const STREAM_ON_IO_OUTPUT: u8 = 0x02;
/// Streaming-off value for [`REG_IO_OUTPUT`]. Tri-states the parallel
/// outputs; the DMA peripheral sees no PCLK edges and idles.
const STREAM_OFF_IO_OUTPUT: u8 = 0x00;

/// Soft-reset payload for [`REG_RESET`].
const SOFT_RESET_PAYLOAD: u8 = 0xF0;

/// Post-power-on settle time before the first I²C read.
///
/// Datasheet specifies ≥50 ms between PWDN release and the first valid
/// register access; 60 ms is the robust default. On the CoreS3 PWDN is
/// tied off so this only covers AVDD / DOVDD ramp.
const POWER_ON_DELAY_MS: u32 = 60;

/// Settle time after a soft reset, before the default-register table is
/// allowed to land. Mirrors the 80 ms wait the upstream ESP-IDF driver
/// uses (`gc0308.c::reset`); shorter waits intermittently corrupt the
/// first register write on some samples.
const SOFT_RESET_DELAY_MS: u32 = 80;

/// Settle time after the default-register table, before format / window
/// switches are accepted. Same 80 ms upstream uses to let the AAA
/// algorithms latch initial state.
const DEFAULTS_SETTLE_MS: u32 = 80;

/// Driver error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// Transport error from the underlying I²C bus.
    I2c(E),
    /// `CHIP_ID` register did not return [`CHIP_ID`]. Contains the byte
    /// that was read.
    BadChipId(u8),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Self::I2c(e)
    }
}

/// Pixel format selector for [`Gc0308::set_format`].
///
/// The low nibble of [`REG_OUTPUT_FMT`] encodes the format. Other modes
/// (raw Bayer, color-bar test pattern) exist in hardware but are not
/// exposed here — the firmware only needs RGB565 for live preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 16-bit RGB565, MSB first. Two bytes per pixel; the `LCD_CAM` DMA
    /// landing buffer is sized at `width * height * 2`.
    Rgb565,
    /// 16-bit `YCbYCr` (YUV422). Useful for grayscale-based image
    /// analysis (Y plane only); not used for live preview.
    Yuv422,
}

impl Format {
    /// Low-nibble value written to [`REG_OUTPUT_FMT`] for this format.
    const fn output_fmt_bits(self) -> u8 {
        match self {
            Self::Rgb565 => 0b0110,
            Self::Yuv422 => 0b0010,
        }
    }
}

/// QVGA subsample configuration (1/2 ratio across both axes).
///
/// The upstream ESP-IDF driver picks this entry from a ratio table when
/// `framesize == FRAMESIZE_QVGA`. Encoded inline because QVGA is the
/// only resolution the firmware needs — VGA / lower resolutions can be
/// added by extending [`Gc0308::set_framesize_qvga`] into a `set_framesize`
/// taking an enum.
const QVGA_SUBSAMPLE_MODE: u8 = 0x22;

/// Reset-window width used during QVGA subsample.
///
/// The chip requires the window to be 8 px larger than the active area
/// on each axis; subsample then halves the output to 320×240.
const QVGA_WINDOW_WIDTH: u16 = 640 + 8;
/// Reset-window height used during QVGA subsample. See
/// [`QVGA_WINDOW_WIDTH`] for why the +8 padding is required.
const QVGA_WINDOW_HEIGHT: u16 = 480 + 8;

/// GC0308 driver handle. Owns the I²C bus + 7-bit address.
pub struct Gc0308<B> {
    /// Underlying I²C bus.
    bus: B,
    /// Resolved 7-bit I²C address.
    address: u8,
}

impl<B: I2c> Gc0308<B> {
    /// Wrap an I²C bus with the fixed GC0308 address.
    ///
    /// Does not touch the bus. Call [`Gc0308::init`] before any other
    /// register access.
    #[must_use]
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            address: ADDRESS,
        }
    }

    /// Resolved 7-bit I²C address. Useful for logging.
    #[must_use]
    pub const fn address(&self) -> u8 {
        self.address
    }

    /// Surrender ownership of the bus. Mirrors the pattern used by
    /// `Axp2101` so callers can chain a single bus through multiple
    /// peripheral handles.
    pub fn into_inner(self) -> B {
        self.bus
    }

    /// Read the `CHIP_ID` register.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn read_chip_id(&mut self) -> Result<u8, Error<B::Error>> {
        let mut buf = [0u8; 1];
        self.bus
            .write_read(self.address, &[REG_CHIP_ID], &mut buf)
            .await?;
        Ok(buf[0])
    }

    /// Full power-on bring-up.
    ///
    /// Steps:
    /// 1. Wait `POWER_ON_DELAY_MS` for the analog rails to settle.
    /// 2. Read and validate the chip ID.
    /// 3. Issue a software reset and wait `SOFT_RESET_DELAY_MS`.
    /// 4. Apply [`DEFAULT_REGS`] verbatim (sensor analog setup,
    ///    AEC/AGC/AWB defaults, gamma curves).
    /// 5. Wait `DEFAULTS_SETTLE_MS` for the AAA algorithms to latch.
    ///
    /// On exit the chip is producing valid frames at full VGA. Callers
    /// typically follow up with [`Gc0308::set_format`],
    /// [`Gc0308::set_framesize_qvga`], and [`Gc0308::set_streaming`] to
    /// reach the streaming configuration the firmware needs.
    ///
    /// # Errors
    ///
    /// - [`Error::BadChipId`] if the chip does not report [`CHIP_ID`].
    /// - [`Error::I2c`] on any bus failure during reset / table load.
    pub async fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<B::Error>> {
        delay.delay_ms(POWER_ON_DELAY_MS).await;

        let id = self.read_chip_id().await?;
        if id != CHIP_ID {
            return Err(Error::BadChipId(id));
        }

        self.write_reg(REG_RESET, SOFT_RESET_PAYLOAD).await?;
        delay.delay_ms(SOFT_RESET_DELAY_MS).await;

        for &(reg, value) in DEFAULT_REGS {
            self.write_reg(reg, value).await?;
        }
        delay.delay_ms(DEFAULTS_SETTLE_MS).await;

        Ok(())
    }

    /// Select the pixel-output format. Switches to page 0 first; the
    /// chip's output-format register lives there regardless of which
    /// page the previous call left active.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_format(&mut self, format: Format) -> Result<(), Error<B::Error>> {
        self.select_page(0).await?;
        self.update_reg_bits(REG_OUTPUT_FMT, 0x0F, format.output_fmt_bits())
            .await
    }

    /// Switch to QVGA (320×240) via 1/2 horizontal+vertical subsample.
    ///
    /// Programs the page-0 reset window to the full sensor (640+8 ×
    /// 480+8) and the page-1 subsample registers to halve both axes.
    /// Ends back on page 0 so callers don't have to think about which
    /// page is current.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_framesize_qvga(&mut self) -> Result<(), Error<B::Error>> {
        self.select_page(0).await?;

        let h_hi = high_byte(QVGA_WINDOW_HEIGHT);
        let h_lo = low_byte(QVGA_WINDOW_HEIGHT);
        let w_hi = high_byte(QVGA_WINDOW_WIDTH);
        let w_lo = low_byte(QVGA_WINDOW_WIDTH);

        for (reg, value) in [
            (REG_ROW_START_H, 0x00),
            (REG_ROW_START_L, 0x00),
            (REG_COL_START_H, 0x00),
            (REG_COL_START_L, 0x00),
            (REG_WIN_HEIGHT_H, h_hi),
            (REG_WIN_HEIGHT_L, h_lo),
            (REG_WIN_WIDTH_H, w_hi),
            (REG_WIN_WIDTH_L, w_lo),
        ] {
            self.write_reg(reg, value).await?;
        }

        self.select_page(1).await?;
        self.update_reg_bits(REG_SUBSAMPLE_EN, 0x80, 0x80).await?;
        self.update_reg_bits(REG_SUBSAMPLE_EN2, 0x01, 0x01).await?;
        for (reg, value) in [
            (REG_SUBSAMPLE_MODE, QVGA_SUBSAMPLE_MODE),
            (REG_SUBSAMPLE_Y0, 0x00),
            (REG_SUBSAMPLE_Y1, 0x00),
            (REG_SUBSAMPLE_UV0, 0x00),
            (REG_SUBSAMPLE_UV1, 0x00),
        ] {
            self.write_reg(reg, value).await?;
        }

        self.select_page(0).await
    }

    /// Gate the parallel-output pads.
    ///
    /// `true` enables PCLK / HSYNC / VSYNC / data; `false` tri-states
    /// them so the DMA peripheral sees no clock edges. CoreS3 doesn't
    /// expose a PWDN pin, so this is the only "stop streaming" knob.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_streaming(&mut self, on: bool) -> Result<(), Error<B::Error>> {
        self.select_page(0).await?;
        let value = if on {
            STREAM_ON_IO_OUTPUT
        } else {
            STREAM_OFF_IO_OUTPUT
        };
        self.write_reg(REG_IO_OUTPUT, value).await
    }

    /// Toggle the horizontal-mirror bit in `CISCTL_MODE1`. Useful when
    /// the camera is mounted rotated relative to the user-facing axis.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_horizontal_mirror(&mut self, on: bool) -> Result<(), Error<B::Error>> {
        self.select_page(0).await?;
        self.update_reg_bits(REG_CISCTL_MODE1, 0x01, u8::from(on))
            .await
    }

    /// Toggle the vertical-flip bit in `CISCTL_MODE1`.
    ///
    /// # Errors
    ///
    /// Propagates any I²C bus error.
    pub async fn set_vertical_flip(&mut self, on: bool) -> Result<(), Error<B::Error>> {
        self.select_page(0).await?;
        let bit = if on { 0x02 } else { 0x00 };
        self.update_reg_bits(REG_CISCTL_MODE1, 0x02, bit).await
    }

    /// Write `value` to a single register on the currently-selected page.
    async fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), Error<B::Error>> {
        self.bus.write(self.address, &[reg, value]).await?;
        Ok(())
    }

    /// Read-modify-write helper for register fields.
    ///
    /// Reads the current byte at `reg`, replaces the bits selected by
    /// `mask` with `value` (also masked), and writes the result back.
    /// `value` must already be aligned to the same bit positions as
    /// `mask`.
    async fn update_reg_bits(
        &mut self,
        reg: u8,
        mask: u8,
        value: u8,
    ) -> Result<(), Error<B::Error>> {
        let mut current = [0u8; 1];
        self.bus
            .write_read(self.address, &[reg], &mut current)
            .await?;
        let new = (current[0] & !mask) | (value & mask);
        self.write_reg(reg, new).await
    }

    /// Switch SCCB page. Page 0 holds the windowing / format / AEC
    /// registers; page 1 holds the subsample tables.
    async fn select_page(&mut self, page: u8) -> Result<(), Error<B::Error>> {
        self.write_reg(REG_PAGE_SELECT, page).await
    }
}

/// High byte of a 16-bit value.
const fn high_byte(value: u16) -> u8 {
    ((value >> 8) & 0xFF) as u8
}

/// Low byte of a 16-bit value.
const fn low_byte(value: u16) -> u8 {
    (value & 0xFF) as u8
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test-only: panicking on unexpected mock state surfaces failures cleanly"
)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use embedded_hal_async::i2c::{Operation, SevenBitAddress};

    /// Host-side I²C mock.
    ///
    /// Records every transaction (address + bytes) in order. For
    /// `write_read` operations the mock answers reads from a per-register
    /// canned-value table, falling back to `0` for any register the
    /// test didn't stage.
    struct MockI2c {
        /// All recorded transactions, in arrival order. Each is the
        /// 7-bit address plus the bytes from a single `Operation`.
        transactions: RefCell<Vec<(u8, Vec<u8>)>>,
        /// Staged read responses keyed by register address. Latest
        /// staging wins.
        read_responses: RefCell<Vec<(u8, u8)>>,
    }

    impl MockI2c {
        fn new() -> Self {
            Self {
                transactions: RefCell::new(Vec::new()),
                read_responses: RefCell::new(Vec::new()),
            }
        }

        fn with_register(self, reg: u8, value: u8) -> Self {
            self.read_responses.borrow_mut().push((reg, value));
            self
        }

        /// Just the bytes from each `Operation::Write`, in order.
        fn write_payloads(&self) -> Vec<Vec<u8>> {
            self.transactions
                .borrow()
                .iter()
                .map(|(_, buf)| buf.clone())
                .collect()
        }

        /// Just the (reg, value) pairs from single-byte register writes.
        /// Two-byte payloads are register writes; everything else (e.g.
        /// the `[reg]` half of a `write_read`) is filtered out.
        fn register_writes(&self) -> Vec<(u8, u8)> {
            self.transactions
                .borrow()
                .iter()
                .filter_map(|(_, buf)| {
                    if buf.len() == 2 {
                        Some((buf[0], buf[1]))
                    } else {
                        None
                    }
                })
                .collect()
        }
    }

    impl embedded_hal_async::i2c::ErrorType for MockI2c {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal_async::i2c::I2c for MockI2c {
        async fn transaction(
            &mut self,
            address: SevenBitAddress,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            let mut last_write_reg: Option<u8> = None;
            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        self.transactions.borrow_mut().push((address, buf.to_vec()));
                        if let Some(&reg) = buf.first() {
                            last_write_reg = Some(reg);
                        }
                    }
                    Operation::Read(buf) => {
                        let value = last_write_reg
                            .and_then(|reg| {
                                self.read_responses
                                    .borrow()
                                    .iter()
                                    .rev()
                                    .find(|(r, _)| *r == reg)
                                    .map(|(_, v)| *v)
                            })
                            .unwrap_or(0);
                        if let Some(slot) = buf.first_mut() {
                            *slot = value;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    /// No-op delay — tests don't actually wait.
    struct NoDelay;
    impl DelayNs for NoDelay {
        async fn delay_ns(&mut self, _ns: u32) {}
    }

    /// Tiny future poller used instead of pulling in an async executor.
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
    fn init_validates_chip_id_and_writes_default_regs_in_order() {
        let bus = MockI2c::new().with_register(REG_CHIP_ID, CHIP_ID);
        let mut cam = Gc0308::new(bus);
        block_on(cam.init(&mut NoDelay)).unwrap();

        let writes = cam.bus.register_writes();
        // First register write is the soft-reset; the rest are the
        // default-register table verbatim.
        assert_eq!(writes[0], (REG_RESET, SOFT_RESET_PAYLOAD));
        assert_eq!(&writes[1..], DEFAULT_REGS);

        // Every transaction targets the GC0308 address.
        for (addr, _) in cam.bus.transactions.borrow().iter() {
            assert_eq!(*addr, ADDRESS);
        }
    }

    #[test]
    fn init_rejects_mismatched_chip_id() {
        let bus = MockI2c::new().with_register(REG_CHIP_ID, 0x42);
        let mut cam = Gc0308::new(bus);
        let err = block_on(cam.init(&mut NoDelay));
        assert!(matches!(err, Err(Error::BadChipId(0x42))), "got {err:?}");
        // No register writes happen before chip-id validation.
        assert!(cam.bus.register_writes().is_empty());
    }

    #[test]
    fn set_format_rgb565_writes_correct_low_nibble() {
        let bus = MockI2c::new().with_register(REG_OUTPUT_FMT, 0xA0);
        let mut cam = Gc0308::new(bus);
        block_on(cam.set_format(Format::Rgb565)).unwrap();

        let writes = cam.bus.register_writes();
        assert_eq!(writes[0], (REG_PAGE_SELECT, 0x00));
        // High nibble of the staged value preserved (0xA0), low nibble
        // replaced with RGB565's 0b0110.
        assert_eq!(writes[1], (REG_OUTPUT_FMT, 0xA6));
    }

    #[test]
    fn set_format_yuv422_writes_correct_low_nibble() {
        let bus = MockI2c::new().with_register(REG_OUTPUT_FMT, 0xA0);
        let mut cam = Gc0308::new(bus);
        block_on(cam.set_format(Format::Yuv422)).unwrap();
        assert_eq!(cam.bus.register_writes()[1], (REG_OUTPUT_FMT, 0xA2));
    }

    #[test]
    fn set_framesize_qvga_programs_window_and_subsample() {
        let bus = MockI2c::new()
            .with_register(REG_SUBSAMPLE_EN, 0x00)
            .with_register(REG_SUBSAMPLE_EN2, 0x00);
        let mut cam = Gc0308::new(bus);
        block_on(cam.set_framesize_qvga()).unwrap();

        let writes = cam.bus.register_writes();
        let expected = [
            (REG_PAGE_SELECT, 0x00),
            (REG_ROW_START_H, 0x00),
            (REG_ROW_START_L, 0x00),
            (REG_COL_START_H, 0x00),
            (REG_COL_START_L, 0x00),
            (REG_WIN_HEIGHT_H, 0x01),
            (REG_WIN_HEIGHT_L, 0xE8),
            (REG_WIN_WIDTH_H, 0x02),
            (REG_WIN_WIDTH_L, 0x88),
            (REG_PAGE_SELECT, 0x01),
            // Subsample-enable RMW writes: staged 0x00 → bits set.
            (REG_SUBSAMPLE_EN, 0x80),
            (REG_SUBSAMPLE_EN2, 0x01),
            (REG_SUBSAMPLE_MODE, QVGA_SUBSAMPLE_MODE),
            (REG_SUBSAMPLE_Y0, 0x00),
            (REG_SUBSAMPLE_Y1, 0x00),
            (REG_SUBSAMPLE_UV0, 0x00),
            (REG_SUBSAMPLE_UV1, 0x00),
            (REG_PAGE_SELECT, 0x00),
        ];
        assert_eq!(writes, expected);
    }

    #[test]
    fn set_streaming_toggles_io_output() {
        let bus = MockI2c::new();
        let mut cam = Gc0308::new(bus);
        block_on(cam.set_streaming(false)).unwrap();
        block_on(cam.set_streaming(true)).unwrap();

        let writes = cam.bus.register_writes();
        // Each call: select page 0, then write IO_OUTPUT.
        assert_eq!(writes[0], (REG_PAGE_SELECT, 0x00));
        assert_eq!(writes[1], (REG_IO_OUTPUT, STREAM_OFF_IO_OUTPUT));
        assert_eq!(writes[2], (REG_PAGE_SELECT, 0x00));
        assert_eq!(writes[3], (REG_IO_OUTPUT, STREAM_ON_IO_OUTPUT));
    }

    #[test]
    fn mirror_and_flip_set_correct_bits() {
        let bus = MockI2c::new().with_register(REG_CISCTL_MODE1, 0x10);
        let mut cam = Gc0308::new(bus);
        block_on(cam.set_horizontal_mirror(true)).unwrap();
        block_on(cam.set_vertical_flip(true)).unwrap();

        let writes = cam.bus.register_writes();
        // After mirror on (preserve 0x10, set bit 0): 0x11.
        assert_eq!(writes[1], (REG_CISCTL_MODE1, 0x11));
        // The mock's staged value is still 0x10 (read-modify-write reads
        // the original each time), so flip-on lands as 0x12.
        assert_eq!(writes[3], (REG_CISCTL_MODE1, 0x12));
    }

    #[test]
    fn read_chip_id_uses_write_read_pattern() {
        let bus = MockI2c::new().with_register(REG_CHIP_ID, CHIP_ID);
        let mut cam = Gc0308::new(bus);
        let id = block_on(cam.read_chip_id()).unwrap();
        assert_eq!(id, CHIP_ID);

        // The chip-id read should only produce one transaction (the
        // `[reg]` write half of write_read records as a single entry).
        let payloads = cam.bus.write_payloads();
        assert_eq!(payloads, vec![vec![REG_CHIP_ID]]);
    }

    #[test]
    fn default_regs_has_expected_size() {
        // Lock the table size so an accidental copy / merge that drops
        // entries surfaces here. Bumping this value requires
        // re-snapshotting `regs::DEFAULT_REGS` from the upstream
        // esp32-camera driver and confirming the chip still streams.
        assert_eq!(DEFAULT_REGS.len(), 239);
    }

    #[test]
    fn default_regs_starts_and_ends_on_page_zero() {
        assert_eq!(DEFAULT_REGS.first(), Some(&(REG_PAGE_SELECT, 0x00)));
        assert_eq!(DEFAULT_REGS.last(), Some(&(REG_PAGE_SELECT, 0x00)));
    }

    #[test]
    fn high_low_byte_round_trip() {
        let value: u16 = 0x12_34;
        assert_eq!(high_byte(value), 0x12);
        assert_eq!(low_byte(value), 0x34);
    }
}
