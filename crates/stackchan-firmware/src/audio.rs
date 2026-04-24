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
//! 6. [TODO PR 2C] enter the sample-processing loop: pop DMA samples,
//!    compute RMS per ~33 ms window, publish via [`AUDIO_RMS_SIGNAL`]
//!
//! This matches esp-bsp's ordering in `bsp_audio_codec_microphone_init`:
//! `bsp_audio_init` (spins up I²S + MCLK) runs *before* `es7210_codec_new`.
//!
//! ## What this PR lands (PR 2B)
//!
//! Steps 1–5 above. The task parks after both codecs are up. Sample
//! processing + RMS publication land in PR 2C, which replaces the park
//! with a real loop. [`AUDIO_RMS_SIGNAL`] stays at its default
//! `AudioRms(0.0)` until then — consumer modifiers see a silent mic,
//! which produces a closed mouth in the downstream binding.

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
    /// Held even though this PR doesn't stream TX, so PR 2C/3 can
    /// wire speaker output without another plumbing change.
    pub dout: GPIO13<'static>,
    /// I²C device handle for the AW88298.
    pub amp_bus: SharedI2c,
    /// I²C device handle for the ES7210.
    pub adc_bus: SharedI2c,
}

/// Audio task entry point.
///
/// Runs the full bring-up sequence (I²S + codecs), then parks
/// indefinitely. PR 2C replaces the park with the real RMS-processing
/// loop.
///
/// Failures at any step log-and-degrade: audio goes silent (signal
/// stays at `AudioRms(0.0)`) but the rest of the avatar keeps running.
pub async fn run_audio_task(p: AudioPeripherals) -> ! {
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
    // I²C. The transfer keeps running for the lifetime of the task.
    let _rx_transfer = match i2s_rx.read_dma_circular_async(rx_buffer) {
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

    defmt::info!("audio: bring-up complete — I²S RX running, RMS processing TODO (PR 2C)");
    park_forever().await
}

/// Infinite sleep for tasks that have nothing else to do. `-> !` so
/// callers can use it in a no-return branch.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
