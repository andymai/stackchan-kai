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

use alloc::boxed::Box;
use aw88298::Aw88298;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, signal::Signal,
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
use stackchan_core::voice::{Priority, Utterance};
use stackchan_tts::{AudioSource, BakedBackend, RenderError, SpeechBackend};

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

/// Wire-side minimum dB for the percentile mapping. `volume_pct = 0`
/// lands here; the mute path (`audio.muted = true` / `POST /mute`)
/// is the actual-silence channel.
const VOLUME_PCT_MIN_DB: i8 = -36;
/// Wire-side maximum dB for the percentile mapping. `volume_pct =
/// 100` lands here (full-scale, no attenuation).
const VOLUME_PCT_MAX_DB: i8 = 0;

/// Linear-in-dB mapping of the wire-format `volume_pct` (0..=100) to
/// AW88298 attenuation.
///
/// dB is already logarithmic in amplitude, so linear interpolation
/// across the dB range gives the perceptual taper without a curve
/// table. Values above 100 saturate at the max — the parser rejects
/// them, but the firmware-side helper stays robust if a future
/// caller bypasses the gate.
#[must_use]
pub fn volume_pct_to_db(pct: u8) -> i8 {
    let pct = i32::from(pct.min(100));
    let span = i32::from(VOLUME_PCT_MAX_DB) - i32::from(VOLUME_PCT_MIN_DB);
    let scaled = i32::from(VOLUME_PCT_MIN_DB) + (span * pct / 100);
    #[allow(clippy::cast_possible_truncation)]
    {
        scaled as i8
    }
}

/// Latest-wins request to apply a new volume to the AW88298.
///
/// Driven by the HTTP `POST /volume` route after the persisted
/// update succeeds, and by the boot path once the SD config snapshot
/// is loaded. The amp-control half of [`run_audio_task`] drains this
/// and calls `Aw88298::set_volume_db` with the mapped dB value.
pub static AUDIO_VOLUME_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Latest-wins request to mute / un-mute the AW88298. Symmetric with
/// [`AUDIO_VOLUME_SIGNAL`]: HTTP `POST /mute` writes after
/// persistence, the amp-control loop applies via `set_muted`.
pub static AUDIO_MUTE_SIGNAL: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// Result of a persisted audio-setting change. Returned by
/// [`persist_volume`] / [`persist_mute`] so the HTTP and BLE
/// control planes can map the outcome onto their own protocol's
/// success / error encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum AudioPersistOutcome {
    /// Config was written, [`crate::net::snapshot`] updated, and the
    /// audio task signaled to apply the new value.
    Persisted,
    /// `CONFIG_SNAPSHOT` was empty — the boot config hasn't been
    /// loaded yet. HTTP maps to `503`, BLE maps to
    /// `WRITE_REQUEST_REJECTED`.
    NoSnapshot,
    /// No SD card mounted. Same status mapping as [`Self::NoSnapshot`].
    NoStorage,
    /// SD write failed. HTTP maps to `500`, BLE maps to
    /// `UNLIKELY_ERROR`. The error is logged via `defmt` at the
    /// callsite that surfaced it.
    WriteFailed,
}

/// Persist a new volume percentile and signal the audio task.
///
/// Mirrors the snapshot-mutex hygiene of the HTTP `POST /volume`
/// path: clone the current snapshot, drop the lock, write to SD,
/// then re-acquire only to install the updated value. Holding the
/// snapshot lock across the SD write would stall every concurrent
/// request that touches `CONFIG_SNAPSHOT` (auth gate, `GET
/// /settings`, BLE provisioning) for the full write duration.
pub async fn persist_volume(level: u8) -> AudioPersistOutcome {
    let Some(current) = crate::storage::CONFIG_SNAPSHOT.lock().await.clone() else {
        return AudioPersistOutcome::NoSnapshot;
    };
    let mut new_config = current;
    new_config.audio.volume_pct = level;
    let write_result =
        crate::storage::with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!("audio: volume persisted (level={=u8})", level);
            crate::net::snapshot::update_audio(new_config.audio);
            *crate::storage::CONFIG_SNAPSHOT.lock().await = Some(new_config);
            AUDIO_VOLUME_SIGNAL.signal(level);
            AudioPersistOutcome::Persisted
        }
        Some(Err(e)) => {
            defmt::warn!("audio: volume write failed ({})", e);
            AudioPersistOutcome::WriteFailed
        }
        None => AudioPersistOutcome::NoStorage,
    }
}

/// Persist a new mute flag and signal the audio task. Symmetric with
/// [`persist_volume`].
pub async fn persist_mute(muted: bool) -> AudioPersistOutcome {
    let Some(current) = crate::storage::CONFIG_SNAPSHOT.lock().await.clone() else {
        return AudioPersistOutcome::NoSnapshot;
    };
    let mut new_config = current;
    new_config.audio.muted = muted;
    let write_result =
        crate::storage::with_storage(|storage| storage.write_config(&new_config)).await;
    match write_result {
        Some(Ok(())) => {
            defmt::info!("audio: mute persisted (muted={=bool})", muted);
            crate::net::snapshot::update_audio(new_config.audio);
            *crate::storage::CONFIG_SNAPSHOT.lock().await = Some(new_config);
            AUDIO_MUTE_SIGNAL.signal(muted);
            AudioPersistOutcome::Persisted
        }
        Some(Err(e)) => {
            defmt::warn!("audio: mute write failed ({})", e);
            AudioPersistOutcome::WriteFailed
        }
        None => AudioPersistOutcome::NoStorage,
    }
}

/// AW88298 settle delay between starting TX DMA (with a buffer of
/// zeros = digital silence) and lifting `HMUTE`. Lets the codec lock
/// onto the I²S clock domain before the output stage goes live so the
/// speaker doesn't pop.
const TX_SETTLE_MS: u32 = 30;

/// Stateless v1 backend instance. The dispatch path consults this
/// directly; multi-backend routing (cloud, on-device) layers in via
/// the `SpeechBackend` trait when those backends land.
const BAKED_BACKEND: BakedBackend = BakedBackend::new();

/// One queued speech item: source + originating priority.
///
/// Priority is read by the TX feeder when the slot becomes current so
/// [`AUDIO_TX_CURRENT_PRIORITY`] tracks what's actually playing.
pub struct SpeechSlot {
    /// PCM source rendered by a [`SpeechBackend`]. Pulled via
    /// [`AudioSource::fill`] until exhausted.
    pub source: Box<dyn AudioSource + Send>,
    /// Original utterance priority. Drives in-flight preemption
    /// decisions on the producer side and tracker updates on the
    /// consumer side.
    pub priority: Priority,
}

/// Speech queue.
///
/// Producers ([`try_dispatch_utterance`] callers — chirp translators in
/// `main.rs`, future modifier-published utterances) enqueue
/// [`SpeechSlot`]s; the TX feeder pops them and pulls samples through
/// [`AudioSource::fill`].
///
/// Capacity 4. Eviction policy when full:
/// - Incoming non-Critical → drop the incoming slot.
/// - Incoming Critical → drain the queue, drop the oldest
///   non-Critical to make room, then re-enqueue the new Critical
///   *at the head* with the survivors behind it. The Critical
///   then plays immediately when `AUDIO_TX_PREEMPT` fires rather
///   than waiting through up to three survivor slots. If every
///   queued slot is also Critical, drop the incoming with a warn
///   log — there's nothing of lower priority to evict without
///   losing equivalent urgency.
pub static AUDIO_TX_QUEUE: Channel<CriticalSectionRawMutex, SpeechSlot, 4> = Channel::new();

/// Hard cap on the eviction-buffer size — matches [`AUDIO_TX_QUEUE`]
/// capacity. A `heapless::Vec` typed at this length lets the
/// eviction path drain the queue without a heap alloc.
const AUDIO_TX_QUEUE_CAPACITY: usize = 4;

/// Discriminant of the [`Priority`] of the source currently playing —
/// `0` (Background) when nothing is playing.
///
/// `Priority` is `#[repr(u8)]` with explicit discriminants so the cast
/// `priority as u8` matches the value seen by [`Priority::partial_cmp`].
///
/// # Memory ordering — single-core invariant
///
/// All accesses to this atomic and to [`AUDIO_TX_PREEMPT`] use
/// [`Ordering::Relaxed`]. That's safe only because esp-rtos runs a
/// single-core embassy executor on the CoreS3: tasks don't preempt
/// each other except at `.await` points, so producer-side
/// `try_dispatch_utterance` (sync, called between `Director::run`
/// frames) and consumer-side `TxSampler::next_sample` (sync, called
/// inside the audio task's `push_with` closure) never interleave.
/// Porting to multi-core (ESP32-P4 or enabling the second Xtensa
/// core) requires revisiting these orderings — at minimum upgrading
/// the producer's PREEMPT store to `Release` and the consumer's swap
/// to `Acquire`.
pub static AUDIO_TX_CURRENT_PRIORITY: AtomicU8 = AtomicU8::new(0);

/// Preemption signal from dispatch path to TX feeder.
///
/// Set by [`try_dispatch_utterance`] when an incoming utterance has
/// strictly higher priority than the source currently playing. The
/// TX feeder observes this on each fill, drops its current source,
/// and clears the flag.
pub static AUDIO_TX_PREEMPT: AtomicBool = AtomicBool::new(false);

/// Reasons [`try_dispatch_utterance`] can fail.
#[derive(Debug)]
#[non_exhaustive]
pub enum DispatchError {
    /// No registered backend reported `can_handle` for the utterance's
    /// content kind.
    NoBackend,
    /// Backend's [`SpeechBackend::render`] returned an error
    /// (asset missing, unsupported phrase, backend unavailable).
    Render(RenderError),
    /// [`AUDIO_TX_QUEUE`] is full and the new slot was dropped.
    QueueFull,
}

impl defmt::Format for DispatchError {
    fn format(&self, f: defmt::Formatter<'_>) {
        // Distinct labels per variant; clippy::match_same_arms sees
        // structurally identical macro expansions for the two unit
        // variants (only the literal differs) and false-flags them.
        #[allow(
            clippy::match_same_arms,
            reason = "labels are distinct strings even though clippy reads the macro arms as identical"
        )]
        match self {
            Self::NoBackend => defmt::write!(f, "NoBackend"),
            Self::Render(e) => defmt::write!(f, "Render({:?})", defmt::Debug2Format(e)),
            Self::QueueFull => defmt::write!(f, "QueueFull"),
        }
    }
}

/// Render an utterance via a registered [`SpeechBackend`] and queue
/// the resulting [`AudioSource`] for TX playback.
///
/// Honors priority preemption: if the new utterance's priority is
/// strictly higher than [`AUDIO_TX_CURRENT_PRIORITY`], sets
/// [`AUDIO_TX_PREEMPT`] so the TX loop drops its in-flight source
/// before pulling from the queue.
///
/// # Errors
///
/// See [`DispatchError`].
pub fn try_dispatch_utterance(utterance: &Utterance) -> Result<(), DispatchError> {
    if !BAKED_BACKEND.can_handle(&utterance.content) {
        return Err(DispatchError::NoBackend);
    }
    let source = BAKED_BACKEND
        .render(utterance)
        .map_err(DispatchError::Render)?;
    let slot = SpeechSlot {
        source,
        priority: utterance.priority,
    };

    // Fast path: queue had room.
    let slot = match AUDIO_TX_QUEUE.try_send(slot) {
        Ok(()) => {
            update_preempt_after_enqueue(utterance.priority);
            return Ok(());
        }
        Err(embassy_sync::channel::TrySendError::Full(rejected)) => rejected,
    };

    // Slow path: queue is full. Only `Critical` incoming attempts to
    // evict; lower priorities preserve in-flight audio.
    if slot.priority != Priority::Critical {
        return Err(DispatchError::QueueFull);
    }

    // Drain FIFO — `try_receive` gives the oldest first — so we can
    // pick the *oldest* non-Critical to drop. Capacity is bounded by
    // [`AUDIO_TX_QUEUE_CAPACITY`]; the buffer never overflows.
    let mut buffer: heapless::Vec<SpeechSlot, AUDIO_TX_QUEUE_CAPACITY> = heapless::Vec::new();
    while let Ok(s) = AUDIO_TX_QUEUE.try_receive() {
        // Push can't fail: queue cap == buffer cap.
        let _ = buffer.push(s);
    }
    let Some(evict_idx) = buffer.iter().position(|s| s.priority != Priority::Critical) else {
        // Every queued slot is Critical too — can't evict without
        // losing equivalent urgency. Restore the queue and drop the
        // incoming with a log line so the operator notices.
        defmt::warn!("audio: queue saturated with Critical; incoming Critical dropped");
        for s in buffer {
            let _ = AUDIO_TX_QUEUE.try_send(s);
        }
        return Err(DispatchError::QueueFull);
    };
    let evicted = buffer.remove(evict_idx);
    defmt::warn!(
        "audio: evicting non-Critical (priority={=u8}) to make room for Critical",
        evicted.priority as u8,
    );
    drop(evicted);
    // Enqueue the Critical at the head, then put the survivors
    // back behind it. `update_preempt_after_enqueue` drops the
    // in-flight source on the next TX fill, and the TX feeder
    // promotes from the queue head — so the Critical plays
    // immediately rather than after the survivor slots.
    AUDIO_TX_QUEUE
        .try_send(slot)
        .map_err(|_| DispatchError::QueueFull)?;
    for s in buffer {
        let _ = AUDIO_TX_QUEUE.try_send(s);
    }
    update_preempt_after_enqueue(Priority::Critical);
    Ok(())
}

/// Set `AUDIO_TX_PREEMPT` if `new_priority` is strictly higher than
/// what's currently playing. Must run *after* the slot is in the
/// queue: if the flag is set before the slot is enqueueable, the TX
/// loop drops the in-flight source and promotes whatever's already
/// at the head of the queue — possibly lower priority than what was
/// dropped.
fn update_preempt_after_enqueue(new_priority: Priority) {
    let current = AUDIO_TX_CURRENT_PRIORITY.load(Ordering::Relaxed);
    if (new_priority as u8) > current {
        AUDIO_TX_PREEMPT.store(true, Ordering::Relaxed);
    }
}

/// Microphone RMS sample, published per render tick.
///
/// Value is the linear-RMS amplitude of the most recent ~33 ms audio
/// window, normalised to `[0.0, 1.0]` against full-scale i16
/// (`32768.0`). A value of `0.01` ≈ -40 dBFS, `0.3` ≈ -10 dBFS.
///
/// The downstream `MouthFromAudio` modifier (PR 3) converts this to
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
/// modifiers (`EmotionFromVoice` / `IntentFromLoud`) on its own chirps.
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

/// TX-side lip-sync hint, published per DMA fill batch (~32 ms cadence).
///
/// While speech is playing, the audio task publishes envelope (and
/// optional viseme) here so `MouthFromAudio` can drive the avatar's
/// mouth from the avatar's own outgoing audio rather than from the
/// gated mic. Latest-wins: the render task reads with `try_take` and
/// stamps `entity.perception.tx_lip_sync`.
///
/// When idle, the audio task does not publish; the render task
/// observes [`AUDIO_TX_PLAYING`] to clear the field on the same
/// transition that ends the gate.
pub static TX_LIP_SYNC_SIGNAL: Signal<CriticalSectionRawMutex, stackchan_core::lipsync::LipSync> =
    Signal::new();

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
    // pre-configure volume from the persisted config (or the audio
    // default if no SD card / snapshot isn't loaded yet) so the
    // un-mute step doesn't go straight to the chip's reset default,
    // then start TX (silent zeros from the freshly-allocated DMA
    // buffer), settle, and finally lift HMUTE — unless the persisted
    // state asks the device to stay muted.
    let boot_audio = crate::storage::CONFIG_SNAPSHOT
        .lock()
        .await
        .as_ref()
        .map_or_else(stackchan_net::config::AudioConfig::default, |cfg| cfg.audio);
    let boot_volume_db = volume_pct_to_db(boot_audio.volume_pct);
    if let Err(e) = amp.set_volume_db(boot_volume_db).await {
        defmt::warn!(
            "audio: AW88298 set_volume_db({=i8}) failed ({:?}); continuing at init default",
            boot_volume_db,
            defmt::Debug2Format(&e)
        );
    } else {
        defmt::debug!(
            "audio: AW88298 volume set to {=i8} dB (boot, from persisted volume_pct={=u8})",
            boot_volume_db,
            boot_audio.volume_pct
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

    // Honour the persisted mute state at boot. The default-config
    // path keeps the prior shipping behaviour (`muted = false` →
    // un-mute on boot); operators who set `audio.muted = true` get
    // a silent boot until they un-mute over HTTP.
    if let Err(e) = amp.set_muted(boot_audio.muted).await {
        defmt::warn!(
            "audio: AW88298 set_muted({=bool}) failed ({:?}); speaker stays at chip-reset state",
            boot_audio.muted,
            defmt::Debug2Format(&e)
        );
    } else if boot_audio.muted {
        defmt::info!("audio: AW88298 muted at boot (persisted audio.muted = true)");
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

    // All three halves are `-> !`, so `join3` itself never resolves;
    // the trailing `park_forever` is unreachable but keeps the
    // function body trivially `-> !` without leaning on never-type
    // coercion. The amp-control half stays in this function so the
    // `amp` binding lives past init — moving it to a separate
    // embassy task would require an alternate I²C bus path.
    embassy_futures::join::join3(
        run_rms_loop(&mut rx_transfer),
        run_tx_loop(&mut tx_transfer),
        async {
            loop {
                use embassy_futures::select::{Either, select};
                match select(AUDIO_VOLUME_SIGNAL.wait(), AUDIO_MUTE_SIGNAL.wait()).await {
                    Either::First(pct) => {
                        let db = volume_pct_to_db(pct);
                        if let Err(e) = amp.set_volume_db(db).await {
                            defmt::warn!(
                                "audio: runtime set_volume_db({=i8}) failed ({:?})",
                                db,
                                defmt::Debug2Format(&e)
                            );
                        } else {
                            defmt::info!(
                                "audio: volume → {=u8}% ({=i8} dB) via runtime signal",
                                pct,
                                db
                            );
                        }
                    }
                    Either::Second(muted) => {
                        if let Err(e) = amp.set_muted(muted).await {
                            defmt::warn!(
                                "audio: runtime set_muted({=bool}) failed ({:?})",
                                muted,
                                defmt::Debug2Format(&e)
                            );
                        } else {
                            defmt::info!("audio: muted → {=bool} via runtime signal", muted);
                        }
                    }
                }
            }
        },
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

/// Sample buffer chunk size for the TX feeder's [`AudioSource::fill`]
/// pulls. 256 samples ≈ 16 ms at 16 kHz — small enough that a
/// preempt flag is observed promptly, large enough that the per-fill
/// overhead is amortised.
const TX_FILL_CHUNK: usize = 256;

/// Iterator-style adapter that pulls samples one at a time from a
/// chain of [`SpeechSlot`]s, refilling its internal buffer from the
/// current source's [`AudioSource::fill`] as needed and pulling fresh
/// slots from [`AUDIO_TX_QUEUE`] when sources exhaust.
///
/// Owns the in-flight [`SpeechSlot`] so it can update
/// [`AUDIO_TX_CURRENT_PRIORITY`] whenever the active source changes,
/// and observes [`AUDIO_TX_PREEMPT`] each call to drop the active
/// source on a preempting utterance.
struct TxSampler {
    /// Slot whose source is currently being drained, if any.
    current: Option<SpeechSlot>,
    /// Pre-fetched samples not yet emitted to the DMA buffer.
    /// Populated by [`AudioSource::fill`] in chunks of up to
    /// [`TX_FILL_CHUNK`].
    buf: [i16; TX_FILL_CHUNK],
    /// Index of the next sample to emit from `buf`.
    buf_pos: usize,
    /// Number of valid samples in `buf` (`buf_pos..buf_len`).
    buf_len: usize,
}

impl TxSampler {
    /// Construct an empty sampler. Idle until a slot is queued.
    const fn new() -> Self {
        Self {
            current: None,
            buf: [0; TX_FILL_CHUNK],
            buf_pos: 0,
            buf_len: 0,
        }
    }

    /// Drop the active source (if any) and clear the local PCM
    /// buffer. Resets [`AUDIO_TX_CURRENT_PRIORITY`] to 0
    /// (`Priority::Background`).
    fn drop_active(&mut self) {
        self.current = None;
        self.buf_pos = 0;
        self.buf_len = 0;
        AUDIO_TX_CURRENT_PRIORITY.store(0, Ordering::Relaxed);
    }

    /// Promote a queued [`SpeechSlot`] to active and publish its
    /// priority. Caller must ensure no source is active.
    fn promote(&mut self, slot: SpeechSlot) {
        AUDIO_TX_CURRENT_PRIORITY.store(slot.priority as u8, Ordering::Relaxed);
        self.current = Some(slot);
    }

    /// Yield the next i16 sample, refilling from the current source
    /// or pulling the next queued slot as needed. Returns `0`
    /// (digital silence) if nothing is queued.
    ///
    /// Preemption is observed only on the slow path (buffer refill),
    /// not per sample. Preempt latency is therefore bounded by the
    /// in-flight buffer length: at most one [`TX_FILL_CHUNK`] worth
    /// of samples (~16 ms at 16 kHz). Per-sample preempt checks
    /// would burn an atomic swap 512× per DMA batch for sub-ms
    /// latency improvement that's well below the perceptual floor.
    fn next_sample(&mut self) -> i16 {
        // Fast path: pop one sample from the pre-fetched buffer.
        if self.buf_pos < self.buf_len {
            let s = self.buf[self.buf_pos];
            self.buf_pos += 1;
            return s;
        }

        // Slow path: observe preempt, then refill from current source,
        // advancing through queued slots if exhausted. Bounded loop —
        // we either get a non-zero fill, or the queue runs dry and we
        // return silence.
        if AUDIO_TX_PREEMPT.swap(false, Ordering::Relaxed) {
            self.drop_active();
        }
        for _ in 0..4 {
            if let Some(slot) = self.current.as_mut() {
                let n = slot.source.fill(&mut self.buf);
                if n > 0 {
                    self.buf_len = n;
                    self.buf_pos = 1;
                    return self.buf[0];
                }
                self.drop_active();
            }
            match AUDIO_TX_QUEUE.try_receive() {
                Ok(slot) => self.promote(slot),
                Err(_) => return 0,
            }
        }
        0
    }

    /// Whether the sampler currently has audio to emit.
    const fn is_active(&self) -> bool {
        self.current.is_some() || self.buf_pos < self.buf_len
    }

    /// Lip-sync hint from the active source, if it supplies one.
    /// `None` falls back to live RMS in the TX loop.
    fn lip_sync_hint(&self) -> Option<stackchan_core::lipsync::LipSync> {
        self.current.as_ref()?.source.lip_sync()
    }
}

/// TX feeder. Drains [`SpeechSlot`]s from [`AUDIO_TX_QUEUE`] and pulls
/// PCM samples through [`AudioSource::fill`]; emits digital silence
/// when idle so the AW88298's I²S receiver stays locked to the clock
/// domain.
///
/// Uses `push_with` so the closure produces exactly as many samples
/// as the DMA tail accepts in this batch — no partial-acceptance
/// bookkeeping outside the closure.
async fn run_tx_loop<BUFFER>(
    tx_transfer: &mut esp_hal::i2s::master::asynch::I2sWriteDmaTransferAsync<'_, BUFFER>,
) -> ! {
    let mut sampler = TxSampler::new();

    loop {
        // Per-batch envelope accumulator. RMS of outgoing samples gives
        // us a backend-agnostic lip-sync envelope when the source
        // can't supply a richer hint via `AudioSource::lip_sync`.
        let mut sum_sq: f32 = 0.0;
        let mut sample_count: u32 = 0;
        let mut had_explicit_lip_sync: Option<stackchan_core::lipsync::LipSync> = None;

        let result = tx_transfer
            .push_with(|buf: &mut [u8]| {
                let pairs = buf.len() / 2;
                for i in 0..pairs {
                    let sample = sampler.next_sample();
                    let s = f32::from(sample);
                    sum_sq += s * s;
                    let bytes = sample.to_le_bytes();
                    buf[i * 2] = bytes[0];
                    buf[i * 2 + 1] = bytes[1];
                }
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "pairs is bounded by the DMA buffer size, well below u32::MAX"
                )]
                let pairs_u32 = pairs as u32;
                sample_count = pairs_u32;
                // Prefer an explicit lip-sync hint from the active
                // source if one is available — backends with viseme
                // or sidecar envelope data outrank live RMS.
                had_explicit_lip_sync = sampler.lip_sync_hint();
                pairs * 2
            })
            .await;

        let active = sampler.is_active();
        AUDIO_TX_PLAYING.store(active, Ordering::Relaxed);

        // Publish lip-sync hint while a source is in flight. When idle,
        // the render task observes AUDIO_TX_PLAYING transitioning to
        // false and clears `entity.perception.tx_lip_sync`.
        if active {
            let hint = had_explicit_lip_sync.unwrap_or_else(|| {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "sample_count <= 2048 (DMA buffer bound); exact in f32"
                )]
                let count_f32 = sample_count as f32;
                let mean_sq = if sample_count == 0 {
                    0.0
                } else {
                    sum_sq / count_f32
                };
                let rms_norm = (mean_sq / FULL_SCALE_SQ).sqrt().min(1.0);
                stackchan_core::lipsync::LipSync::envelope(rms_norm)
            });
            TX_LIP_SYNC_SIGNAL.signal(hint);
        }

        if let Err(e) = result {
            defmt::warn!(
                "audio: TX DMA push error ({:?}); backing off",
                defmt::Debug2Format(&e)
            );
            Timer::after(Duration::from_millis(10)).await;
        }
    }
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
