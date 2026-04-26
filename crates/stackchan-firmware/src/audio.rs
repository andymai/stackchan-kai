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
//! 6. Configure AW88298 output: set boot volume via `set_volume_db`,
//!    start the TX DMA on a silent-zero buffer, settle, lift `HMUTE`.
//!    Speaker is now live and clocking.
//! 7. Run RX + TX loops concurrently inside the same embassy task via
//!    `embassy_futures::join`:
//!    - RX (`run_rms_loop`): pop DMA samples, compute linear RMS over
//!      each [`RMS_WINDOW_SAMPLES`]-sample window, normalise against
//!      full-scale i16, publish on [`AUDIO_RMS_SIGNAL`].
//!    - TX (`run_tx_loop`): play queued [`AudioClip`]s back-to-back,
//!      filling with digital silence between/after clips so the
//!      AW88298 stays clock-locked. Higher-level code (low-battery
//!      alerts, pickup chirps, startle chirps, etc.) enqueues clips
//!      via [`try_enqueue_clip`] and the typed
//!      [`try_enqueue_wake_chirp`] / [`try_enqueue_pickup_chirp`] /
//!      [`try_enqueue_startle_chirp`] / [`try_enqueue_low_battery_alert`]
//!      helpers.
//!
//! This matches esp-bsp's ordering in `bsp_audio_codec_microphone_init`:
//! `bsp_audio_init` (spins up I²S + MCLK) runs *before* `es7210_codec_new`.
//!
//! Failures inside the loops log-and-degrade rather than parking:
//! - RX DMA pop error → publish `AudioRms(0.0)` (closed mouth) and
//!   resync after a short backoff.
//! - TX DMA push error → back off briefly and retry. Speaker may
//!   click but won't fall silent permanently.
//! - TX DMA start failure → fall through to RX-only mode (RMS loop
//!   runs without speaker output).

use aw88298::Aw88298;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Channel, TrySendError},
    signal::Signal,
};
use embassy_time::{Delay, Duration, Timer};
use es7210::Es7210;
use esp_hal::{
    dma_circular_buffers,
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

/// Size of the TX DMA circular buffer, in bytes.
///
/// 4 KiB ≈ 128 ms of audio at 16 kHz × 16-bit mono. Smaller than the
/// RX buffer because the TX feeder runs on a tighter schedule (it's
/// pushing per-frame samples in close-to-real-time) and 128 ms is
/// plenty of headroom for the embassy scheduler's worst-case latency.
const TX_DMA_BYTES: usize = 4 * 1024;

/// AW88298 attenuation, in dB, applied at TX bring-up. -18 dB is
/// audible-but-not-startling for a 1 W desktop speaker; combined with
/// a -12 dBFS digital tone the boot greeting plays at ≈ -30 dB.
const BOOT_VOLUME_DB: i8 = -18;

/// AW88298 settle delay between starting TX DMA (with a buffer of
/// zeros = digital silence) and lifting `HMUTE`. Lets the codec lock
/// onto the I²S clock domain before the output stage goes live so the
/// speaker doesn't pop.
const TX_SETTLE_MS: u32 = 30;

/// One-cycle 1 kHz sine table at 16 kHz sample rate. 16 samples per
/// cycle. Amplitude `8192 ≈ -12 dBFS`, picked so the AW88298 output
/// stage stays well clear of the digital ceiling. Pre-computed at
/// compile time so the TX feeder is `sin()`-free at runtime (and
/// `libm`-free in this firmware crate).
const SINE_1KHZ_CYCLE: [i16; 16] = [
    0, 3135, 5793, 7568, 8192, 7568, 5793, 3135, 0, -3135, -5793, -7568, -8192, -7568, -5793, -3135,
];

/// One-cycle 2 kHz sine table at 16 kHz sample rate. 8 samples per
/// cycle. Same -12 dBFS amplitude as [`SINE_1KHZ_CYCLE`]; intended for
/// alert-style beeps (low-battery, error chirps) where the higher
/// pitch makes it distinct from the boot greeting.
const SINE_2KHZ_CYCLE: [i16; 8] = [0, 5793, 8192, 5793, 0, -5793, -8192, -5793];

/// One-cycle 4 kHz sine table at 16 kHz sample rate. 4 samples per
/// cycle (the highest pitch we can produce cleanly without going
/// past the Nyquist limit). Used as the top of the
/// [`PICKUP_CHIRP`] rising sweep.
const SINE_4KHZ_CYCLE: [i16; 4] = [0, 8192, 0, -8192];

/// 8-sample silence cycle for [`AudioClip`]-encoded gaps. Used between
/// successive beeps in [`try_enqueue_low_battery_alert`] so the user
/// hears two distinct pulses rather than one long tone.
const SILENCE_CYCLE: [i16; 8] = [0; 8];

/// Boot greeting: 500 ms of 1 kHz sine. Available for explicit
/// playback via the `audio-bench` example; not auto-enqueued at boot.
///
/// `500 cycles × 16 samples = 8 000 samples = 500 ms` at the 16 kHz
/// sample rate. Tweak `cycles` to change duration without touching
/// the table.
pub const BOOT_GREETING: AudioClip = AudioClip {
    samples: &SINE_1KHZ_CYCLE,
    cycles: 500,
};

/// Single-clip "wake" chirp: 100 ms of 1 kHz sine. Audibly soft but
/// distinct from the boot greeting (which is 5× longer); enqueued
/// when [`stackchan_core::modifiers::WakeOnVoice`] just fired.
///
/// `100 cycles × 16 samples = 1 600 samples = 100 ms`.
pub const WAKE_CHIRP: AudioClip = AudioClip {
    samples: &SINE_1KHZ_CYCLE,
    cycles: 100,
};

/// First leg of the pickup chirp: 50 ms of 2 kHz.
const PICKUP_CHIRP_LO: AudioClip = AudioClip {
    samples: &SINE_2KHZ_CYCLE,
    cycles: 100,
};
/// Second leg of the pickup chirp: 50 ms of 4 kHz. Played
/// back-to-back with [`PICKUP_CHIRP_LO`] for an upward sweep that
/// matches the "Surprised!" emotion fire from
/// [`stackchan_core::modifiers::IntentReflex`].
const PICKUP_CHIRP_HI: AudioClip = AudioClip {
    samples: &SINE_4KHZ_CYCLE,
    cycles: 200,
};

/// Single beep used inside [`try_enqueue_low_battery_alert`]. 100 ms
/// of 2 kHz; two of these separated by silence form the full alert.
const LOW_BATTERY_BEEP: AudioClip = AudioClip {
    samples: &SINE_2KHZ_CYCLE,
    cycles: 200,
};

/// First leg of the camera-mode-enter chirp: 50 ms of 1 kHz.
/// Distinct from the pickup chirp (2 kHz → 4 kHz, brighter sweep) and
/// from the wake chirp (single 1 kHz tone) by the descending two-tone
/// pattern formed with [`CAMERA_ENTER_CHIRP_HI`].
const CAMERA_ENTER_CHIRP_LO: AudioClip = AudioClip {
    samples: &SINE_1KHZ_CYCLE,
    cycles: 50,
};
/// Second leg of the camera-mode-enter chirp: 80 ms of 2 kHz —
/// upward two-tone "doot-DEE" that signals "preview is now on
/// screen." Plays back-to-back with [`CAMERA_ENTER_CHIRP_LO`].
const CAMERA_ENTER_CHIRP_HI: AudioClip = AudioClip {
    samples: &SINE_2KHZ_CYCLE,
    cycles: 160,
};

/// First leg of the camera-mode-exit chirp: 80 ms of 2 kHz.
/// Inverted ordering of the enter chirp — descending "DEE-doot" that
/// signals "back to avatar."
const CAMERA_EXIT_CHIRP_HI: AudioClip = AudioClip {
    samples: &SINE_2KHZ_CYCLE,
    cycles: 160,
};
/// Second leg of the camera-mode-exit chirp: 50 ms of 1 kHz.
const CAMERA_EXIT_CHIRP_LO: AudioClip = AudioClip {
    samples: &SINE_1KHZ_CYCLE,
    cycles: 50,
};
/// 80 ms gap between the two low-battery beeps. Silence stored as a
/// short cycle table looped many times — keeps the tx feeder code
/// uniform (everything goes through clip playback).
const LOW_BATTERY_GAP: AudioClip = AudioClip {
    samples: &SILENCE_CYCLE,
    cycles: 160,
};

/// PCM audio clip queued for TX playback.
///
/// Stored as a `&'static [i16]` buffer (one cycle of a tone, or a
/// pre-baked sample) plus a `cycles` count. The TX feeder plays
/// through `samples` `cycles` times back-to-back, then transitions
/// to silence (or the next queued clip).
///
/// `cycles = 0` plays nothing — the clip is consumed but produces
/// zero output samples. Useful for testing the queue without making
/// noise.
///
/// # Examples
///
/// ```ignore
/// // 500 ms of a 1 kHz tone using a 16-sample cycle table.
/// const TONE: &[i16] = &[/* one 1 kHz cycle at 16 kHz */];
/// let clip = AudioClip::new(TONE, 500);
/// audio::AUDIO_TX_QUEUE.try_send(clip).ok();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AudioClip {
    /// Sample buffer. Played through `cycles` times, in order.
    /// Conventional encoding: 16-bit signed mono at
    /// [`SAMPLE_RATE_HZ`].
    pub samples: &'static [i16],
    /// How many times to loop through `samples`. `1` = one-shot.
    pub cycles: u32,
}

impl AudioClip {
    /// Construct a clip from a sample buffer + cycle count.
    #[must_use]
    pub const fn new(samples: &'static [i16], cycles: u32) -> Self {
        Self { samples, cycles }
    }

    /// Construct a one-shot clip (plays through `samples` exactly once).
    #[must_use]
    pub const fn one_shot(samples: &'static [i16]) -> Self {
        Self { samples, cycles: 1 }
    }
}

/// TX clip queue.
///
/// Producers (any task) enqueue [`AudioClip`]s; the audio task plays
/// them back-to-back, falling back to digital silence when empty.
/// Capacity 4 fits a few queued alerts without blocking the producer;
/// when the queue is full, [`try_enqueue_clip`] returns `Err` and the
/// caller can drop the clip rather than block.
pub static AUDIO_TX_QUEUE: Channel<CriticalSectionRawMutex, AudioClip, 4> = Channel::new();

/// Enqueue a clip for TX playback. Non-blocking; returns `Err` if the
/// queue is full so the caller can drop the clip rather than wait.
///
/// # Errors
///
/// Returns [`TrySendError::Full`] if [`AUDIO_TX_QUEUE`] has 4 clips
/// already queued.
pub fn try_enqueue_clip(clip: AudioClip) -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(clip)
}

/// Enqueue the canonical low-battery alert: two 100 ms 2 kHz beeps
/// separated by an 80 ms gap. Three queued clips total.
///
/// Best-effort: if the queue fills mid-sequence, the partially-queued
/// alert plays as far as it got and the helper returns the failure.
/// The render-loop caller logs once and continues; partial alerts
/// are unlikely in practice (the queue starts empty between fires).
///
/// # Errors
///
/// Returns the first [`TrySendError::Full`] encountered. Earlier
/// clips that did fit are not rolled back — they will play.
pub fn try_enqueue_low_battery_alert() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(LOW_BATTERY_BEEP)?;
    AUDIO_TX_QUEUE.try_send(LOW_BATTERY_GAP)?;
    AUDIO_TX_QUEUE.try_send(LOW_BATTERY_BEEP)?;
    Ok(())
}

/// Enqueue the wake chirp: 100 ms of 1 kHz. One queued clip.
///
/// `stackchan_core::modifiers::WakeOnVoice` sets
/// `entity.voice.chirp_request = Some(ChirpKind::Wake)` on the tick it
/// flips emotion to `Happy`; the render task drains that and calls this
/// to play a confirmation tone.
///
/// # Errors
///
/// Returns [`TrySendError::Full`] if [`AUDIO_TX_QUEUE`] is full.
pub fn try_enqueue_wake_chirp() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(WAKE_CHIRP)
}

/// Enqueue the pickup chirp: 50 ms of 2 kHz then 50 ms of 4 kHz —
/// an upward sweep that matches the "Surprised!" emotion fire from
/// [`stackchan_core::modifiers::IntentReflex`]. Two queued clips.
///
/// Best-effort, same partial-queue caveat as
/// [`try_enqueue_low_battery_alert`].
///
/// # Errors
///
/// Returns the first [`TrySendError::Full`] encountered. The first
/// clip, if it queued successfully, will play.
pub fn try_enqueue_pickup_chirp() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(PICKUP_CHIRP_LO)?;
    AUDIO_TX_QUEUE.try_send(PICKUP_CHIRP_HI)?;
    Ok(())
}

/// Enqueue the startle chirp: 50 ms of 4 kHz — sharp single tone.
///
/// `stackchan_core::modifiers::StartleOnLoud` sets
/// `entity.voice.chirp_request = Some(ChirpKind::Startle)` on the
/// rising edge across the loud-RMS threshold; the render task drains
/// that and calls this. Reuses [`PICKUP_CHIRP_HI`] (the high half of
/// the pickup sweep) for a deliberately sharp, singular reaction —
/// distinct from the pickup chirp's two-tone sweep.
///
/// # Errors
///
/// Returns [`TrySendError::Full`] if [`AUDIO_TX_QUEUE`] is full.
pub fn try_enqueue_startle_chirp() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(PICKUP_CHIRP_HI)
}

/// Enqueue the camera-mode-enter chirp: 50 ms of 1 kHz then 80 ms of
/// 2 kHz — an upward two-tone "doot-DEE" signalling that the preview
/// is now on screen. Two queued clips.
///
/// Pair with [`crate::camera::CAMERA_MODE_SIGNAL`] = `true`
/// transitions in `render_task`. Best-effort, same partial-queue
/// caveat as [`try_enqueue_pickup_chirp`].
///
/// # Errors
///
/// Returns the first [`TrySendError::Full`] encountered.
pub fn try_enqueue_camera_mode_enter() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(CAMERA_ENTER_CHIRP_LO)?;
    AUDIO_TX_QUEUE.try_send(CAMERA_ENTER_CHIRP_HI)?;
    Ok(())
}

/// Enqueue the camera-mode-exit chirp: 80 ms of 2 kHz then 50 ms of
/// 1 kHz — a descending two-tone "DEE-doot" that mirrors the enter
/// chirp inverted. Two queued clips.
///
/// Pair with [`crate::camera::CAMERA_MODE_SIGNAL`] = `false`
/// transitions in `render_task`. Best-effort, same partial-queue
/// caveat as [`try_enqueue_pickup_chirp`].
///
/// # Errors
///
/// Returns the first [`TrySendError::Full`] encountered.
pub fn try_enqueue_camera_mode_exit() -> Result<(), TrySendError<AudioClip>> {
    AUDIO_TX_QUEUE.try_send(CAMERA_EXIT_CHIRP_HI)?;
    AUDIO_TX_QUEUE.try_send(CAMERA_EXIT_CHIRP_LO)?;
    Ok(())
}

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

/// `true` while the TX feeder has a clip in flight.
///
/// Set / cleared once per DMA push (~32 ms cadence) inside
/// [`run_tx_loop`]. The render task reads this each frame and gates
/// `entity.perception.audio_rms` to `None` while playing — without
/// the gate, the speaker output would re-trigger sound-reactive
/// modifiers (`WakeOnVoice` / `StartleOnLoud`) on its own chirps.
///
/// Pair the read with a [`TX_GATE_TAIL_MS`] tail window so the mic
/// doesn't pick up the speaker's residual response immediately after
/// playback ends.
pub static AUDIO_TX_PLAYING: AtomicBool = AtomicBool::new(false);

/// How long after TX playback ends to keep gating the mic, in ms.
///
/// `150 ms` covers the speaker's mechanical decay plus a small margin
/// for the AW88298's anti-pop ramp. Tunable on-device via the
/// audio-bench if it turns out to be wrong.
pub const TX_GATE_TAIL_MS: u64 = 150;

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
    /// Wired into the I²S TX DMA for speaker output.
    pub dout: GPIO13<'static>,
    /// I²C device handle for the AW88298.
    pub amp_bus: SharedI2c,
    /// I²C device handle for the ES7210.
    pub adc_bus: SharedI2c,
}

/// Audio task entry point.
///
/// Runs the full bring-up sequence (I²S + codecs + TX un-mute), then
/// runs the RX RMS loop and TX feeder concurrently via
/// `embassy_futures::join`. Output: a 1 kHz boot greeting on the
/// speaker, an `AudioRms` stream on `AUDIO_RMS_SIGNAL`.
///
/// Failures during bring-up that take out the I²S itself park the
/// task. Failures that take out only TX fall through to RX-only mode.
/// Failures inside either loop log-and-resync rather than parking.
#[allow(
    clippy::too_many_lines,
    reason = "single bring-up sequence — splitting into helpers fragments \
              the I²S → codec → TX ordering invariants for negligible benefit"
)]
pub async fn run_audio_task(mut p: AudioPeripherals) -> ! {
    defmt::debug!(
        "audio: I²S0 bring-up — {=u32} Hz / {=u8}-bit mono, MCLK {=u32} Hz",
        SAMPLE_RATE_HZ,
        BIT_DEPTH_BITS,
        MCLK_HZ,
    );

    // RX + TX DMA buffers live on the task's stack via the
    // `dma_circular_buffers!` macro — must be the *circular* variant
    // since both halves drive `read_dma_circular_async` /
    // `write_dma_circular_async`. The non-circular `dma_buffers!`
    // sized only enough descriptors for a one-shot transfer; circular
    // mode wraps and consumes more, surfacing as
    // `DmaError(OutOfDescriptors)` at TX-start with the audible
    // symptom of a cascade of `DmaError(Late)` retries on the RX side.
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) =
        dma_circular_buffers!(RX_DMA_BYTES, TX_DMA_BYTES);

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

    // Split the I²S into its RX + TX halves. RX claims the BCLK and
    // WS pins (the master peripheral generates both clocks; routing
    // them once is enough — TX shares the same physical pads).
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(p.bclk)
        .with_ws(p.ws)
        .with_din(p.din)
        .build(rx_descriptors);
    let i2s_tx = i2s.i2s_tx.with_dout(p.dout).build(tx_descriptors);

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
    defmt::debug!("audio: I²S RX DMA running — MCLK / BCLK / LRCK clocking");

    // MCLK settle. ES7210 datasheet says "a few ms" but empirically
    // (and per esp-adf), the chip can take longer to latch the clock
    // domain on cold-boot — give it 200 ms.
    Timer::after(Duration::from_millis(200)).await;

    let mut delay = Delay;
    let mut amp = Aw88298::new(p.amp_bus);
    match amp.init(&mut delay).await {
        Ok(()) => defmt::debug!(
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

    // AW88298 output-stage bring-up. With TX DMA not yet started, the
    // TX line carries no clocked data — the codec is on standby. We
    // pre-configure volume so the un-mute step doesn't go straight to
    // the chip's reset default, then start TX (silent zeros from the
    // freshly-allocated DMA buffer), settle, and finally lift HMUTE.
    if let Err(e) = amp.set_volume_db(BOOT_VOLUME_DB).await {
        defmt::warn!(
            "audio: AW88298 set_volume_db({=i8}) failed ({:?}); continuing at init default",
            BOOT_VOLUME_DB,
            defmt::Debug2Format(&e)
        );
    } else {
        defmt::debug!(
            "audio: AW88298 volume set to {=i8} dB (boot default)",
            BOOT_VOLUME_DB
        );
    }

    let mut tx_transfer = match i2s_tx.write_dma_circular_async(tx_buffer) {
        Ok(t) => t,
        Err(e) => {
            defmt::error!(
                "audio: TX DMA start failed ({:?}); continuing without speaker output",
                defmt::Debug2Format(&e)
            );
            // RX still works — run the RMS loop without TX. Diverges
            // (`-> !`), so control flow doesn't fall through.
            run_rms_loop(&mut rx_transfer).await
        }
    };
    defmt::debug!("audio: I²S TX DMA running — feeding silence");

    Timer::after(Duration::from_millis(u64::from(TX_SETTLE_MS))).await;

    if let Err(e) = amp.set_muted(false).await {
        defmt::warn!(
            "audio: AW88298 un-mute failed ({:?}); speaker stays muted",
            defmt::Debug2Format(&e)
        );
    } else {
        defmt::info!("audio: AW88298 un-muted — speaker live");
    }

    // The TX queue is the single source of TX content from here on.
    // Nothing is enqueued at boot — clips (boot greetings, low-battery
    // alerts, pickup chirps) are pushed by their respective triggers.
    defmt::info!(
        "audio: bring-up complete — RX RMS loop ({=u32}-sample windows) + TX feeder (clip-queue driven)",
        RMS_WINDOW_SAMPLES,
    );

    // Both halves are `-> !`, so `join` itself never resolves; the
    // trailing `park_forever` is unreachable but keeps the function
    // body trivially `-> !` without leaning on never-type coercion.
    embassy_futures::join::join(
        run_rms_loop(&mut rx_transfer),
        run_tx_loop(&mut tx_transfer),
    )
    .await;
    park_forever().await
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

    // Rate-limit the DMA-pop warning. The `Late` failure mode is
    // self-perpetuating in `read_dma_circular_async` — once the
    // descriptor chain wraps past the read pointer, every subsequent
    // pop also returns `Late` until the transfer is rebuilt. Logging
    // every retry at `warn` floods the defmt link (~100 lines/s) and
    // hides every other log. Emit one `warn` per `LOG_EVERY_N_DMA_ERRS`
    // pops so the symptom stays visible without drowning the channel.
    let mut consecutive_dma_errs: u32 = 0;
    /// Log only every Nth consecutive pop error.
    #[allow(clippy::items_after_statements)]
    const LOG_EVERY_N_DMA_ERRS: u32 = 200;

    loop {
        // Watchdog heartbeat fires once per audio-task iteration, not
        // once per completed RMS window. The DmaError(Late) path
        // recovers via `continue` without producing an RMS sample, but
        // the audio task itself is still alive — that's what the
        // watchdog cares about. Producing-RMS-correctly is a separate
        // concern that surfaces via the `audio: RMS …` debug logs.
        crate::watchdog::AUDIO.beat();
        let n = match rx_transfer.pop(&mut scratch).await {
            Ok(n) => {
                consecutive_dma_errs = 0;
                n
            }
            Err(e) => {
                if consecutive_dma_errs.is_multiple_of(LOG_EVERY_N_DMA_ERRS) {
                    defmt::warn!(
                        "audio: DMA pop error ({:?}); publishing silence and resyncing (next log in {=u32} pops)",
                        defmt::Debug2Format(&e),
                        LOG_EVERY_N_DMA_ERRS,
                    );
                }
                consecutive_dma_errs = consecutive_dma_errs.saturating_add(1);
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

/// In-flight playback of one [`AudioClip`]. Tracks the cursor inside
/// `samples` and how many full cycles remain.
struct ClipPlayback {
    /// Held by reference; the clip itself lives in `.rodata` (or any
    /// `'static` location).
    samples: &'static [i16],
    /// Index of the next sample to emit on a `next_sample()` call.
    cursor: usize,
    /// Cycles left to play through `samples`. Decremented to zero
    /// when the cursor wraps past the end of the slice; once at zero
    /// the clip is done and `next_sample()` returns `None`.
    cycles_remaining: u32,
}

impl ClipPlayback {
    /// Construct a fresh playback cursor at the start of `clip`.
    const fn new(clip: AudioClip) -> Self {
        Self {
            samples: clip.samples,
            cursor: 0,
            cycles_remaining: clip.cycles,
        }
    }

    /// Yield the next sample, or `None` if the clip is done. An empty
    /// `samples` slice or `cycles = 0` yields `None` immediately.
    fn next_sample(&mut self) -> Option<i16> {
        if self.cycles_remaining == 0 || self.samples.is_empty() {
            return None;
        }
        let s = self.samples[self.cursor];
        self.cursor += 1;
        if self.cursor >= self.samples.len() {
            self.cursor = 0;
            self.cycles_remaining -= 1;
        }
        Some(s)
    }
}

/// TX feeder. Pulls [`AudioClip`]s off [`AUDIO_TX_QUEUE`] and plays
/// them back-to-back; emits digital silence when the queue is empty
/// so the AW88298's I²S receiver stays locked to the clock domain.
///
/// Mid-batch transitions: when one clip ends partway through a push
/// buffer, the feeder immediately checks the queue for the next clip
/// and continues without an audible gap. If nothing is queued, the
/// remainder of the buffer is filled with zeros and the loop tries
/// again on the next push.
///
/// Uses `push_with` so the closure produces exactly as many samples
/// as the DMA tail accepts in this batch — no partial-acceptance
/// bookkeeping outside the closure.
async fn run_tx_loop<BUFFER>(
    tx_transfer: &mut esp_hal::i2s::master::asynch::I2sWriteDmaTransferAsync<'_, BUFFER>,
) -> ! {
    // The currently-playing clip, if any. `None` = silence.
    let mut current: Option<ClipPlayback> = None;

    loop {
        let result = tx_transfer
            .push_with(|buf: &mut [u8]| {
                let pairs = buf.len() / 2;
                for i in 0..pairs {
                    let sample = next_sample_with_chaining(&mut current);
                    let bytes = sample.to_le_bytes();
                    buf[i * 2] = bytes[0];
                    buf[i * 2 + 1] = bytes[1];
                }
                pairs * 2
            })
            .await;

        // Publish playback state once per DMA buffer (~32 ms). The
        // render task uses this + TX_GATE_TAIL_MS to suppress
        // self-trigger of sound-reactive modifiers.
        AUDIO_TX_PLAYING.store(current.is_some(), Ordering::Relaxed);

        if let Err(e) = result {
            defmt::warn!(
                "audio: TX DMA push error ({:?}); backing off",
                defmt::Debug2Format(&e)
            );
            Timer::after(Duration::from_millis(10)).await;
        }
    }
}

/// Yield one TX sample. If the current clip is exhausted, transition
/// straight into the next queued clip (if any) so consecutive clips
/// play without a silence gap. Returns `0` if no clip is playable.
fn next_sample_with_chaining(current: &mut Option<ClipPlayback>) -> i16 {
    // Up to two iterations: first attempt with `current`, second
    // attempt with whatever was just pulled from the queue. Bounded
    // because each new clip yields at least one sample (or `None` if
    // its slice is empty / cycles == 0, in which case we fall
    // through to silence).
    for _ in 0..2 {
        if let Some(s) = current.as_mut().and_then(ClipPlayback::next_sample) {
            return s;
        }
        *current = AUDIO_TX_QUEUE.try_receive().ok().map(ClipPlayback::new);
        if current.is_none() {
            return 0;
        }
    }
    // Shouldn't reach here for non-degenerate clips. Treat any
    // pathological zero-yield clip as silence.
    0
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
    defmt::debug!("audio: I²C bus scan starting (0x08..=0x77)");
    let mut found: u32 = 0;
    for addr in 0x08_u8..=0x77 {
        let mut buf = [0u8; 1];
        if bus.write_read(addr, &[0x00], &mut buf).await.is_ok() {
            defmt::debug!(
                "audio: I²C 0x{=u8:02X} ACK (first byte @ reg 0x00 = 0x{=u8:02X})",
                addr,
                buf[0]
            );
            found += 1;
        }
    }
    defmt::debug!("audio: I²C bus scan complete — {=u32} devices ACKed", found);
}
