//! Audio boot-up and signal plumbing.
//!
//! This module owns the firmware's audio control path: bringing the
//! two codecs ([`aw88298`] speaker amp, [`es7210`] mic ADC) online at
//! the fixed 16 kHz / 16-bit mono shape the avatar's voice-reactive
//! pipeline expects, and exposing the channel that will carry
//! microphone-RMS values to consumer modifiers.
//!
//! ## What's wired today (PR 2A)
//!
//! - [`bringup`] applies both codec initialisation sequences over the
//!   shared I²C bus. After it returns, the amp is muted but I²S-ready,
//!   and the ADC is power-on with mic1+2 active.
//! - [`AUDIO_RMS_SIGNAL`] is the embassy `Signal` the audio task will
//!   publish to. Declared at module scope so consumer tasks (the
//!   `MouthOpenAudio` modifier in PR 3) can import it even before the
//!   producer runs.
//!
//! ## What's pending (PR 2B)
//!
//! - ESP32-S3 I²S0 peripheral setup: master mode, MCLK out on GPIO0,
//!   BCLK on GPIO34, LRCK on GPIO33, DOUT on GPIO13, DIN on GPIO14,
//!   12.288 MHz MCLK, 16 kHz LRCK, Philips 16-bit mono slot.
//! - DMA ring buffers + [`run_audio_loop`] streaming mic samples,
//!   computing RMS per render-tick window, publishing via
//!   [`AUDIO_RMS_SIGNAL`].
//! - Speaker-side sine-tone generator (driven by emotion transitions
//!   in PR 3's modifier stack).
//!
//! Until PR 2B lands, [`run_audio_loop`] is a park loop that emits a
//! one-time boot log and then yields forever. The signal stays at its
//! default `AudioRms(0.0)` value, which produces a closed mouth in the
//! downstream modifier — a graceful degradation.

use aw88298::Aw88298;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_async::i2c::I2c;
use es7210::Es7210;

/// Audio configuration constants the I²S side must match once wired up.

/// Target sample rate for both codecs.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Master clock the ESP32-S3 I²S peripheral feeds both codecs.
///
/// `12.288 MHz = 256 × 48 kHz = 768 × 16 kHz`. Using 768× oversample
/// at 16 kHz keeps the ES7210 coefficient table row we ported in
/// `crates/es7210/src/lib.rs` valid (`coeff_div[{12288000, 16000}]`).
pub const MCLK_HZ: u32 = 12_288_000;
/// Sample bit-depth. AW88298 spec is 16-bit; ES7210 ADC runs 24-bit
/// internally but truncates to 16-bit over I²S at this slot width.
pub const BIT_DEPTH_BITS: u8 = 16;

/// Microphone RMS sample, published per render tick.
///
/// Value is the linear-RMS amplitude of the most recent ~33 ms
/// audio window, normalised to `[0.0, 1.0]` against full-scale i16
/// (`32768.0`). A value of `0.01` ≈ -40 dBFS, a value of `0.3` ≈ -10 dBFS.
///
/// The downstream [`MouthOpenAudio`] modifier (PR 3) converts this to
/// `dB` + applies an attack/release envelope before writing to the
/// avatar's `mouth_open` field. Keeping the raw value as the channel
/// payload lets the modifier stack own the feel-tuning.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioRms(pub f32);

/// Microphone RMS channel. Single-producer (the audio task) /
/// multiple-consumer (modifier pipeline + eventual diagnostics).
///
/// Uses `Signal` rather than `Channel` because consumers always want
/// the latest value, never a backlog — if the modifier misses a frame,
/// the next one should reflect current mic state, not queued history.
pub static AUDIO_RMS_SIGNAL: Signal<CriticalSectionRawMutex, AudioRms> = Signal::new();

/// Audio-subsystem bring-up error.
///
/// Split from the per-driver `Error<E>` types so `main.rs` can log a
/// single enum — the firmware boot path treats "audio failed" as
/// degrade-gracefully, not panic.
#[derive(Debug, defmt::Format)]
pub enum BringupError {
    /// AW88298 rejected its init sequence. Wrapped in `defmt::Display2Format`
    /// to stay `no_std`-compatible without pulling the `E` generic up.
    AmpInit,
    /// ES7210 rejected its init sequence.
    AdcInit,
}

/// Apply both codec initialisation sequences over the shared I²C bus.
///
/// After this returns:
///
/// - AW88298 is configured for 16 kHz / 16-bit Philips I²S, muted,
///   boost disabled, volume at the esp-adf default (-24 dB).
/// - ES7210 is configured for 12.288 MHz → 16 kHz, mic1+2 active at
///   ~+30 dB, mic3+4 gated.
///
/// Does **not** start any streaming — that's the I²S peripheral's job
/// (pending PR 2B). The amp stays muted until a future un-mute call
/// after the I²S master clocks are stable.
///
/// ## Known issue pre-PR 2B: ES7210 needs MCLK to answer I²C
///
/// The ES7210 gates its I²C state machine on the `MCLK` clock
/// domain, so until the ESP32-S3 I²S peripheral is configured and
/// outputting MCLK on GPIO0, this call NACKs at the chip-ID probe
/// (`BadChipId(0xFF, 0xFF)`). This matches esp-bsp's ordering in
/// `bsp_audio_codec_microphone_init`, which calls `bsp_audio_init`
/// (spins up I²S + MCLK) *before* `es7210_codec_new`. PR 2B fixes
/// the order; until then, expect a warn-level "ES7210 bring-up
/// failed" at boot on real hardware. AW88298 has no such
/// dependency — it comes up fine today.
///
/// # Errors
///
/// - [`BringupError::AmpInit`] if the AW88298 fails its sequence
///   (usually a NACK from an un-released `RST` pin).
/// - [`BringupError::AdcInit`] if the ES7210 fails its sequence
///   (expected until PR 2B — see note above).
pub async fn bringup<B: I2c>(bus_amp: B, bus_adc: B) -> Result<(), BringupError> {
    let mut delay = Delay;
    let mut amp = Aw88298::new(bus_amp);
    amp.init(&mut delay).await.map_err(|e| {
        defmt::error!("audio: AW88298 init: {}", defmt::Debug2Format(&e));
        BringupError::AmpInit
    })?;
    defmt::info!(
        "audio: AW88298 ready — I²S 16 kHz mono, muted, boost off (un-mute via audio task)"
    );

    let mut adc = Es7210::new(bus_adc);
    adc.init(&mut delay).await.map_err(|e| {
        defmt::error!("audio: ES7210 init: {}", defmt::Debug2Format(&e));
        BringupError::AdcInit
    })?;
    defmt::info!("audio: ES7210 ready — 12.288 MHz MCLK / 16 kHz / mic1+2 on");
    Ok(())
}

/// Audio task entry point.
///
/// **Placeholder until PR 2B lands.** Logs one boot message, then
/// parks the task forever. [`AUDIO_RMS_SIGNAL`] stays at its default
/// `AudioRms(0.0)` — consumer modifiers see a silent mic until the
/// I²S peripheral starts streaming.
///
/// The signature already takes the pieces the real task will need:
/// once PR 2B wires I²S, the body becomes an `embassy-futures::select`
/// between RX DMA completions and the render-tick cadence.
pub async fn run_audio_loop() -> ! {
    defmt::info!(
        "audio: task parked — I²S peripheral wiring pending (PR 2B); RMS signal stays at 0.0"
    );
    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
