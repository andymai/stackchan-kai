//! Audio boot-up, I²S peripheral, and signal plumbing.
//!
//! Owns the firmware's audio control path: ESP32-S3 I²S0 master setup
//! (MCLK on GPIO0, BCLK GPIO34, LRCK GPIO33, DIN GPIO14, DOUT GPIO13),
//! both codec bring-ups over the shared I²C bus, and the embassy
//! `Signal` channel the downstream avatar modifier consumes.
//!
//! ## Bring-up ordering
//!
//! ES7210 gates its I²C state machine on the external MCLK domain —
//! until the I²S peripheral is clocking MCLK on GPIO0, every I²C
//! transaction against the ADC returns `0xFF` (NACK). So the boot
//! order inside [`run_audio_task`] is:
//!
//! 1. Configure I²S0 with MCLK + BCLK + LRCK + DIN pins
//! 2. Start RX DMA on a circular buffer — this is what actually drives
//!    clocks out of the I²S peripheral onto the pins
//! 3. Small settle delay
//! 4. Bring up AW88298 (doesn't need MCLK but comes up with the pair)
//! 5. Bring up ES7210 (now responds — MCLK is flowing)
//! 6. Enter the sample-processing loop: pop DMA samples, compute
//!    linear RMS over each [`RMS_WINDOW_SAMPLES`]-sample window
//!    (~33 ms at 16 kHz, one render frame at 30 FPS), normalise
//!    against full-scale i16, publish on [`AUDIO_RMS_SIGNAL`].
//!
//! This matches esp-bsp's ordering in `bsp_audio_codec_microphone_init`:
//! `bsp_audio_init` (spins up I²S + MCLK) runs *before* `es7210_codec_new`.
//!
//! Failures inside the RMS loop log-and-degrade: a DMA pop error
//! publishes `AudioRms(0.0)` (silent mic → closed mouth downstream)
//! and resumes after a short backoff rather than parking the task.

use aw88298::Aw88298;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Timer};
use es7210::Es7210;
use esp_hal::{
    dma_buffers,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    peripherals::{DMA_CH0, GPIO0, GPIO13, GPIO14, GPIO33, GPIO34, I2S0},
    time::Rate,
};
use micromath::F32Ext as _;

use crate::board::SharedI2c;

/// Target sample rate for both codecs.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Master clock the ESP32-S3 I²S peripheral feeds both codecs.
///
/// `12.288 MHz = 256 × 48 kHz = 768 × 16 kHz`. 768× oversample at
/// 16 kHz keeps the ES7210 coefficient table row we ported (`coeff_div[
/// {12288000, 16000}]`) valid.
pub const MCLK_HZ: u32 = 12_288_000;
/// Sample bit-depth. AW88298 spec is 16-bit; ES7210 ADC runs 24-bit
/// internally but truncates to 16-bit over I²S at this slot width.
pub const BIT_DEPTH_BITS: u8 = 16;

/// Size of the RX DMA circular buffer, in bytes.
///
/// Sized to hold roughly ~100 ms of audio at 16 kHz × 16-bit mono
/// (32 kB/s data rate → ~3.2 kB per 100 ms). Larger buffers tolerate
/// longer consumer-side processing gaps; smaller ones tighten latency.
/// 8 KiB is a comfortable middle ground and fits in internal SRAM
/// (DMA-capable) without impacting the PSRAM framebuffer budget.
const RX_DMA_BYTES: usize = 8 * 1024;

/// RMS analysis window, in samples. ~33 ms at 16 kHz — matches one
/// render frame at 30 FPS, so the consumer in `main.rs` sees a fresh
/// value on (almost) every tick.
const RMS_WINDOW_SAMPLES: u32 = SAMPLE_RATE_HZ * 33 / 1000;

/// Bytes per `pop` from the circular DMA tail. Sample-aligned (×2 = ×16-bit).
/// 256 bytes ≈ 8 ms of audio: small enough that we don't sit idle
/// waiting for a giant chunk, large enough to amortise pop overhead.
const POP_SCRATCH_BYTES: usize = 256;

/// Diagnostic log cadence inside the RMS loop. One info line every
/// ~2 s of audio (60 windows × 33 ms ≈ 1.98 s) — enough to eyeball mic
/// activity over RTT without flooding the link.
const LOG_EVERY_N_WINDOWS: u32 = 60;

/// `i16::MIN.unsigned_abs()² = 32768² = 2³⁰`. Pre-computed because the
/// per-window normalisation `(mean_sq / FULL_SCALE_SQ).sqrt()` runs
/// 30 times a second; the value is exact in f32 (mantissa fits 24
/// bits, this needs 1).
const FULL_SCALE_SQ: f32 = 32768.0 * 32768.0;

/// Microphone RMS sample, published per render tick.
///
/// Value is the linear-RMS amplitude of the most recent ~33 ms audio
/// window, normalised to `[0.0, 1.0]` against full-scale i16
/// (`32768.0`). A value of `0.01` ≈ -40 dBFS, `0.3` ≈ -10 dBFS.
///
/// The downstream `MouthOpenAudio` modifier (PR 3) converts this to
/// dB + applies an attack/release envelope before writing to the
/// avatar's `mouth_open` field.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioRms(pub f32);

/// Microphone RMS channel. Single-producer (audio task) /
/// multi-consumer (modifier pipeline + diagnostics).
///
/// `Signal` semantics (latest-wins, no backlog) match the consumer's
/// need: if the modifier misses a frame, the next one should reflect
/// current mic state, not queued history.
pub static AUDIO_RMS_SIGNAL: Signal<CriticalSectionRawMutex, AudioRms> = Signal::new();

/// Audio task peripherals, grouped so `main.rs` spawns the task with
/// a single `Spawner::spawn` call rather than a 9-argument function.
pub struct AudioPeripherals {
    /// I²S0 controller.
    pub i2s: I2S0<'static>,
    /// DMA channel for the I²S RX path.
    pub dma: DMA_CH0<'static>,
    /// Master-clock output pin. CoreS3 schematic: `GPIO0`.
    pub mclk: GPIO0<'static>,
    /// Bit-clock output pin. CoreS3 schematic: `GPIO34`.
    pub bclk: GPIO34<'static>,
    /// Word-select (LRCK) output pin. CoreS3 schematic: `GPIO33`.
    pub ws: GPIO33<'static>,
    /// Data-in pin (ES7210 → ESP32-S3). CoreS3 schematic: `GPIO14`.
    pub din: GPIO14<'static>,
    /// Data-out pin (ESP32-S3 → AW88298). CoreS3 schematic: `GPIO13`.
    /// Held for future TX streaming (speaker output) — wiring it through
    /// now avoids another plumbing change when that lands.
    pub dout: GPIO13<'static>,
    /// I²C device handle for the AW88298.
    pub amp_bus: SharedI2c,
    /// I²C device handle for the ES7210.
    pub adc_bus: SharedI2c,
}

/// Audio task entry point.
///
/// Runs the full bring-up sequence (I²S + codecs), then enters the
/// per-window RMS loop that pops DMA samples and publishes
/// [`AUDIO_RMS_SIGNAL`].
///
/// Failures during bring-up park the task — audio goes silent
/// (`AudioRms(0.0)`) but the rest of the avatar keeps running.
/// Failures inside the loop log-and-resync rather than parking.
pub async fn run_audio_task(mut p: AudioPeripherals) -> ! {
    defmt::info!(
        "audio: I²S0 bring-up — {=u32} Hz / {=u8}-bit mono, MCLK {=u32} Hz",
        SAMPLE_RATE_HZ,
        BIT_DEPTH_BITS,
        MCLK_HZ,
    );

    // RX DMA buffers live on the task's stack via the `dma_buffers!`
    // macro. The buffer outlives the transfer because the task itself
    // never returns.
    let (rx_buffer, rx_descriptors, _tx_buffer, _tx_descriptors) = dma_buffers!(RX_DMA_BYTES, 0);

    let i2s = match I2s::new(
        p.i2s,
        p.dma,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::MONO),
    ) {
        Ok(i2s) => i2s.into_async(),
        Err(e) => {
            defmt::error!(
                "audio: I²S0 config rejected ({:?}); task parking",
                defmt::Debug2Format(&e)
            );
            park_forever().await;
        }
    };
    // `with_mclk` connects the internal MCLK signal to `GPIO0`. MCLK
    // only *flows* once a DMA transfer starts on the RX or TX side,
    // so we start RX below.
    let i2s = i2s.with_mclk(p.mclk);

    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(p.bclk)
        .with_ws(p.ws)
        .with_din(p.din)
        .build(rx_descriptors);
    // `p.dout` is held for future TX streaming (PR 2C / 3); suppress
    // the unused warning without dropping the pin.
    let _ = p.dout;

    // Start the RX DMA circular transfer. From this point MCLK / BCLK
    // / LRCK are all clocking on their pins; ES7210 can now answer
    // I²C. The transfer keeps running for the lifetime of the task —
    // the RMS loop below pops samples off its tail.
    let mut rx_transfer = match i2s_rx.read_dma_circular_async(rx_buffer) {
        Ok(t) => t,
        Err(e) => {
            defmt::error!(
                "audio: RX DMA start failed ({:?}); task parking",
                defmt::Debug2Format(&e)
            );
            park_forever().await;
        }
    };
    defmt::info!("audio: I²S RX DMA running — MCLK / BCLK / LRCK clocking");

    // MCLK settle. ES7210 datasheet says "a few ms" but empirically
    // (and per esp-adf), the chip can take longer to latch the clock
    // domain on cold-boot — give it 200 ms.
    Timer::after(Duration::from_millis(200)).await;

    let mut delay = Delay;
    let mut amp = Aw88298::new(p.amp_bus);
    match amp.init(&mut delay).await {
        Ok(()) => defmt::info!(
            "audio: AW88298 ready — I²S 16 kHz mono, muted, boost off (un-mute deferred)"
        ),
        Err(e) => {
            defmt::error!(
                "audio: AW88298 init failed ({:?}); task parking",
                defmt::Debug2Format(&e)
            );
            park_forever().await;
        }
    }

    // Diagnostic: scan the shared I²C bus to see which addresses ACK.
    // Useful for isolating "chip missing / power fault" from "chip
    // alive but firmware bug." Expected ACKs on the CoreS3:
    //   0x10/11 BMM150, 0x23 LTR-553, 0x34 AXP2101, 0x36 AW88298,
    //   0x38 FT6336U, 0x40 ES7210, 0x51 BM8563, 0x58 AW9523,
    //   0x68/69 BMI270, 0x6F PY32.
    scan_i2c_bus(&mut p.adc_bus).await;

    // ES7210 probe loop. If the first chip-ID read NACKs, retry up to
    // 5 × 100 ms — accommodates codecs that take a while to wake from
    // power-on once MCLK is present.
    let mut adc = Es7210::new(p.adc_bus);
    let mut attempt = 0;
    let init_result = loop {
        match adc.init(&mut delay).await {
            Ok(()) => break Ok(()),
            Err(e) if attempt < 5 => {
                defmt::warn!(
                    "audio: ES7210 init attempt {=u8} failed ({:?}); retrying",
                    attempt,
                    defmt::Debug2Format(&e),
                );
                attempt += 1;
                Timer::after(Duration::from_millis(100)).await;
            }
            Err(e) => break Err(e),
        }
    };
    match init_result {
        Ok(()) => defmt::info!(
            "audio: ES7210 ready — 12.288 MHz MCLK / 16 kHz / mic1+2 on (attempt {=u8})",
            attempt
        ),
        Err(e) => {
            defmt::error!(
                "audio: ES7210 init failed after retries ({:?}); task parking",
                defmt::Debug2Format(&e)
            );
            park_forever().await;
        }
    }

    defmt::info!(
        "audio: bring-up complete — entering RMS loop ({=u32}-sample / ~33 ms windows)",
        RMS_WINDOW_SAMPLES,
    );

    run_rms_loop(&mut rx_transfer).await
}

/// Per-window RMS computation loop. Pops samples off the running
/// circular DMA transfer, accumulates `sum(sample²)` for
/// [`RMS_WINDOW_SAMPLES`] samples, then publishes the normalised RMS
/// on [`AUDIO_RMS_SIGNAL`].
///
/// Pulled out of [`run_audio_task`] mainly to keep that function a
/// readable bring-up sequence; this fn owns its own state machine
/// (accumulator, byte-carry, log throttle) and never returns.
async fn run_rms_loop<BUFFER>(
    rx_transfer: &mut esp_hal::i2s::master::asynch::I2sReadDmaTransferAsync<'_, BUFFER>,
) -> ! {
    let mut scratch = [0u8; POP_SCRATCH_BYTES];
    // Per-window accumulator. Reset on every publish.
    let mut sum_sq: f32 = 0.0;
    let mut count: u32 = 0;
    // Carries the low byte of an i16 sample whose two bytes straddle a
    // pop boundary — `chunks_exact(2)` would otherwise drop it.
    let mut byte_carry: Option<u8> = None;
    // Wrapping window counter; only used to throttle the diagnostic log.
    let mut window_no: u32 = 0;

    loop {
        let n = match rx_transfer.pop(&mut scratch).await {
            Ok(n) => n,
            Err(e) => {
                defmt::warn!(
                    "audio: DMA pop error ({:?}); publishing silence and resyncing",
                    defmt::Debug2Format(&e)
                );
                AUDIO_RMS_SIGNAL.signal(AudioRms(0.0));
                sum_sq = 0.0;
                count = 0;
                byte_carry = None;
                Timer::after(Duration::from_millis(10)).await;
                continue;
            }
        };
        if n == 0 {
            continue;
        }

        let mut bytes = &scratch[..n];

        // Reassemble the sample whose low byte was carried over from the
        // previous pop, if any.
        if let Some(low) = byte_carry.take() {
            if let Some((&high, rest)) = bytes.split_first() {
                accumulate(&mut sum_sq, &mut count, i16::from_le_bytes([low, high]));
                bytes = rest;
            } else {
                byte_carry = Some(low);
                continue;
            }
        }

        let mut chunks = bytes.chunks_exact(2);
        for pair in &mut chunks {
            accumulate(
                &mut sum_sq,
                &mut count,
                i16::from_le_bytes([pair[0], pair[1]]),
            );

            if count >= RMS_WINDOW_SAMPLES {
                // `count` is bounded by `RMS_WINDOW_SAMPLES` (528 in
                // the current config) — well below f32's 24-bit mantissa
                // limit, so the `as f32` cast is exact.
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "count <= RMS_WINDOW_SAMPLES ≪ 2²⁴, exact in f32"
                )]
                let mean_sq = sum_sq / (count as f32);
                // i16::MIN.abs() is one larger than i16::MAX, so a
                // saturated negative sample can yield rms_norm slightly
                // above 1.0 (~3e-5). Clamp so consumers can rely on the
                // documented [0, 1] contract.
                let rms_norm = (mean_sq / FULL_SCALE_SQ).sqrt().min(1.0);
                AUDIO_RMS_SIGNAL.signal(AudioRms(rms_norm));

                sum_sq = 0.0;
                count = 0;
                window_no = window_no.wrapping_add(1);

                if window_no.is_multiple_of(LOG_EVERY_N_WINDOWS) {
                    defmt::info!("audio: RMS {=f32} (linear, full-scale = 1.0)", rms_norm);
                }
            }
        }

        // Stash an odd trailing byte for the next pop.
        byte_carry = chunks.remainder().first().copied();
    }
}

/// Add one i16 sample's contribution to the running window accumulator.
/// `f32::from(i16)` is exact (24-bit mantissa > 16-bit range).
#[inline]
fn accumulate(sum_sq: &mut f32, count: &mut u32, sample: i16) {
    let s = f32::from(sample);
    *sum_sq += s * s;
    *count += 1;
}

/// Infinite sleep for tasks that have nothing else to do. `-> !` so
/// callers can use it in a no-return branch.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Scan all 7-bit I²C addresses (`0x08..=0x77`) and log which ones
/// ACK. Uses a 1-byte register read (`reg = 0`) — chips that require a
/// specific opening register may still ACK the address but NACK the
/// read; that's fine, we only care about the address-level ACK.
///
/// Purely diagnostic. Silent on addresses that don't respond; chatty
/// (one info log per ACK) on addresses that do.
async fn scan_i2c_bus<B: embedded_hal_async::i2c::I2c>(bus: &mut B) {
    defmt::info!("audio: I²C bus scan starting (0x08..=0x77)");
    let mut found: u32 = 0;
    for addr in 0x08_u8..=0x77 {
        let mut buf = [0u8; 1];
        if bus.write_read(addr, &[0x00], &mut buf).await.is_ok() {
            defmt::info!(
                "audio: I²C 0x{=u8:02X} ACK (first byte @ reg 0x00 = 0x{=u8:02X})",
                addr,
                buf[0]
            );
            found += 1;
        }
    }
    defmt::info!("audio: I²C bus scan complete — {=u32} devices ACKed", found);
}
