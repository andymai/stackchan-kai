//! GC0308 camera task — DVP capture into PSRAM-backed ping-pong buffers.
//!
//! Owns the ESP32-S3 `LCD_CAM` peripheral in slave mode (the camera
//! self-clocks from an on-board oscillator on the CoreS3 — XCLK is not
//! routed to a GPIO) plus the 11 DVP pins (D0..D7 + PCLK + HREF +
//! VSYNC). The camera's I²C / SCCB interface shares the existing
//! [`SharedI2c`] bus; the [`gc0308`] driver handles register-level
//! init, format selection, and stream gating.
//!
//! ## Always-on capture for tracking
//!
//! The task streams continuously from boot. The block-grid `tracker`
//! runs on every captured frame; the resulting [`TrackingObservation`]
//! is published on [`CAMERA_TRACKING_SIGNAL`] so the engine's
//! Perception/Cognition modifiers can drive head + eye motion toward
//! whatever moved.
//!
//! [`CAMERA_MODE_SIGNAL`] is **display-only** — it controls whether
//! the LCD shows the camera preview vs. the avatar; it does not gate
//! capture or tracking. Tracking observations flow regardless of
//! preview state.
//!
//! ## DMA strategy
//!
//! Two QVGA RGB565 frame buffers (320 × 240 × 2 = 150 KiB each) live in
//! PSRAM via `Box::leak`. DMA descriptors live in internal SRAM (via
//! `static mut`). The task alternates `receive(buf_a) → wait →
//! receive(buf_b) → wait`, publishing the most-recently completed
//! buffer index on [`CAMERA_FRAME_SIGNAL`] so the render task can blit
//! it to the LCD on the next render tick. There is a brief window
//! between `wait` returning and the next `receive` where incoming
//! frames are dropped — at 30 FPS the gap is small enough not to be
//! perceptible.
//!
//! ## Toggle UX
//!
//! The render task drives the visible mode change: when
//! [`CAMERA_MODE_SIGNAL`] is `true` it skips the avatar blit and instead
//! reads the latest camera buffer pointer; when `false` it resumes
//! avatar rendering. Modifiers keep ticking either way so the avatar's
//! emotion state evolves in the background.
//!
//! ## Unsafe usage
//!
//! Per the workspace policy, this module gates `unsafe` to a single
//! purpose: bridging the camera-task-owned scratch slot
//! (`&'static mut [u8]`) into the cross-task `&'static [u8]` view we
//! publish on [`CAMERA_FRAME_SIGNAL`]. The borrow checker can't
//! express "owner has mut access between frames; readers see immutable
//! access during a single frame" without a runtime mutex; we accept a
//! raw-pointer hand-off backed by `Box::leak`ed PSRAM. The two
//! ping-pong scratch slots strictly alternate, so the render task
//! never reads a slot the camera task is currently writing.

#![allow(
    unsafe_code,
    reason = "raw-pointer hand-off of leaked PSRAM scratch slots between \
              the camera and render tasks; see module docs"
)]

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    dma::DmaRxBuf,
    lcd_cam::{
        ByteOrder, LcdCam,
        cam::{Camera, Config as CamConfig, EofMode},
    },
    peripherals::{
        DMA_CH1, GPIO15, GPIO16, GPIO38, GPIO39, GPIO40, GPIO41, GPIO42, GPIO45, GPIO46, GPIO47,
        GPIO48, LCD_CAM,
    },
    time::Rate,
};
use stackchan_core::{TrackingMotion, TrackingObservation};
use tracker::{Motion, Tracker, TrackerConfig};

use crate::board::SharedI2c;

/// Camera frame width — QVGA (subsampled 1/2 from VGA).
pub const FRAME_WIDTH: u32 = 320;
/// Camera frame height — QVGA.
pub const FRAME_HEIGHT: u32 = 240;
/// Bytes per RGB565 pixel.
pub const BYTES_PER_PIXEL: usize = 2;
/// Total bytes in one captured QVGA RGB565 frame (= 153 600).
pub const FRAME_BYTES: usize = (FRAME_WIDTH as usize) * (FRAME_HEIGHT as usize) * BYTES_PER_PIXEL;

/// Number of ping-pong buffers. Two is enough: while DMA fills one,
/// the render task reads the other.
pub const BUFFER_COUNT: usize = 2;

/// Per-buffer descriptor count. Each ESP32-S3 DMA descriptor covers up
/// to 4095 bytes, so a 150 KiB buffer needs `ceil(153_600 / 4095) =
/// 38`. We round up to 40 to leave headroom for alignment.
const DESCRIPTORS_PER_BUFFER: usize = 40;

/// Pixel-clock frequency target.
///
/// Matches M5Stack's reference firmware (`xclk_freq_hz = 20_000_000`
/// in `M5CoreS3::GC0308`). The CoreS3 camera self-clocks at this
/// rate from an on-board oscillator; we configure the `LCD_CAM`
/// peripheral expecting that `PCLK` rate.
pub const PIXEL_CLOCK_HZ: u32 = 20_000_000;

/// LCD preview-mode toggle published by [`crate::button`].
///
/// `true` = LCD shows the camera preview. `false` = LCD shows the
/// avatar. The camera task itself ignores this signal — capture +
/// tracking are always-on. The render task is the sole consumer.
/// `Signal` semantics (latest-wins, no backlog) are correct here: a
/// rapid double-toggle just lands on whichever value the producer
/// signalled last, matching the user's intent.
pub static CAMERA_MODE_SIGNAL: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// Capture-still trigger published by the render task.
///
/// The render task signals this when the user taps in camera mode
/// (a tap-while-camera-mode = "save this frame"). The camera task
/// snapshots the most recently completed buffer and emits a stats +
/// thumbnail-strip log over RTT. No flash storage; capture is RTT-only.
pub static CAMERA_CAPTURE_REQUEST: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Most-recently completed camera frame as a static slice.
///
/// The camera task copies each just-filled DMA buffer into one of two
/// scratch slots (alternating per ping-pong index) and publishes the
/// resulting `&'static [u8]` here. The render task drains this every
/// tick; while in camera mode it blits the latest received slice
/// directly to the LCD. `try_take` is non-blocking and "no fresh
/// frame" is the common case at 30 FPS render / 30 FPS capture parity
/// — the mode integration falls back to the previously-cached pointer.
///
/// Slices are `153_600` bytes each (QVGA RGB565). The scratch slots are
/// PSRAM-backed; the copy from DMA buffer to scratch costs ~2 ms per
/// frame at PSRAM bandwidth, in exchange for race-free cross-task
/// reads (the DMA peripheral never writes into a slot the render
/// task is reading).
pub static CAMERA_FRAME_SIGNAL: Signal<CriticalSectionRawMutex, &'static [u8]> = Signal::new();

/// Latest tracker observation, camera task → engine.
///
/// The camera task runs `tracker::Tracker::step` on every captured
/// frame and publishes the [`TrackingObservation`] result here.
/// Latest-wins: the engine drains this once per render tick into
/// `entity.perception.tracking`. Always-on regardless of
/// [`CAMERA_MODE_SIGNAL`] — preview gating (display) is independent
/// from tracker analysis (always running).
pub static CAMERA_TRACKING_SIGNAL: Signal<CriticalSectionRawMutex, TrackingObservation> =
    Signal::new();

/// Translate the firmware-side `tracker::Motion` into the
/// engine-side `TrackingMotion` mirror enum. Pure mapping; no logic.
const fn motion_to_engine(m: Motion) -> TrackingMotion {
    match m {
        Motion::Warmup => TrackingMotion::Warmup,
        Motion::Tracking => TrackingMotion::Tracking,
        Motion::Holding => TrackingMotion::Holding,
        Motion::Returning => TrackingMotion::Returning,
        Motion::GlobalEvent => TrackingMotion::GlobalEvent,
    }
}

/// Camera task peripherals, grouped so `main.rs` spawns the task with
/// a single `Spawner::spawn` call rather than a 14-argument function.
///
/// Mirrors [`crate::audio::AudioPeripherals`].
pub struct CameraPeripherals {
    /// `LCD_CAM` peripheral (camera half).
    pub lcd_cam: LCD_CAM<'static>,
    /// DMA channel reserved for the camera RX path. Audio claimed
    /// `DMA_CH0`; we take `DMA_CH1` — the ESP32-S3 has 5 GDMA channels.
    pub dma: DMA_CH1<'static>,
    /// I²C handle to talk to the GC0308 over SCCB. Shares the internal
    /// I²C0 bus (address `0x21`).
    pub i2c: SharedI2c,
    /// PCLK input. CoreS3 schematic: `GPIO45`.
    pub pclk: GPIO45<'static>,
    /// HREF (DE) input. CoreS3 schematic: `GPIO38`.
    pub href: GPIO38<'static>,
    /// VSYNC input. CoreS3 schematic: `GPIO46`.
    pub vsync: GPIO46<'static>,
    /// DVP D0. CoreS3 schematic: `GPIO39`.
    pub d0: GPIO39<'static>,
    /// DVP D1. CoreS3 schematic: `GPIO40`.
    pub d1: GPIO40<'static>,
    /// DVP D2. CoreS3 schematic: `GPIO41`.
    pub d2: GPIO41<'static>,
    /// DVP D3. CoreS3 schematic: `GPIO42`.
    pub d3: GPIO42<'static>,
    /// DVP D4. CoreS3 schematic: `GPIO15`.
    pub d4: GPIO15<'static>,
    /// DVP D5. CoreS3 schematic: `GPIO16`.
    pub d5: GPIO16<'static>,
    /// DVP D6. CoreS3 schematic: `GPIO48`.
    pub d6: GPIO48<'static>,
    /// DVP D7. CoreS3 schematic: `GPIO47`.
    pub d7: GPIO47<'static>,
}

/// Camera task entry point.
///
/// Runs the GC0308 SCCB init + `LCD_CAM` peripheral construction once
/// at boot, lifts the SCCB stream gate, then enters the always-on
/// capture loop. Each completed frame is copied to a scratch slot,
/// published on [`CAMERA_FRAME_SIGNAL`] for the render task's preview
/// blit, and fed through `tracker::Tracker::step` — the resulting
/// [`TrackingObservation`] lands on [`CAMERA_TRACKING_SIGNAL`] for the
/// engine's Perception/Cognition modifiers.
///
/// [`CAMERA_MODE_SIGNAL`] is read by the render task only and decides
/// whether the LCD shows the camera preview vs. the avatar; it does
/// not gate capture or tracking.
///
/// Init failures (chip-id mismatch, peripheral construction error)
/// log at `error` and park the task — the avatar continues running
/// without the camera, matching the existing `audio` / `imu` failure
/// patterns. Runtime DMA errors log at `warn` and resync after a
/// brief backoff.
#[allow(
    clippy::too_many_lines,
    reason = "single bring-up sequence — splitting into helpers fragments \
              the SCCB init → peripheral construction → capture loop ordering"
)]
pub async fn run_camera_task(p: CameraPeripherals) -> ! {
    use embassy_time::Delay;
    use gc0308::{Format, Gc0308};

    defmt::info!(
        "camera: GC0308 bring-up — QVGA {=u32}x{=u32} RGB565 @ ~30 FPS, PCLK {=u32} Hz",
        FRAME_WIDTH,
        FRAME_HEIGHT,
        PIXEL_CLOCK_HZ,
    );

    let mut sensor = Gc0308::new(p.i2c);
    if let Err(e) = sensor.init(&mut Delay).await {
        defmt::error!(
            "camera: GC0308 init failed ({:?}); camera disabled",
            defmt::Debug2Format(&e),
        );
        park_forever().await;
    }
    if let Err(e) = sensor.set_format(Format::Rgb565).await {
        defmt::error!(
            "camera: set_format(RGB565) failed ({:?}); camera disabled",
            defmt::Debug2Format(&e),
        );
        park_forever().await;
    }
    if let Err(e) = sensor.set_framesize_qvga().await {
        defmt::error!(
            "camera: set_framesize_qvga failed ({:?}); camera disabled",
            defmt::Debug2Format(&e),
        );
        park_forever().await;
    }
    // Streaming is always-on for tracking. CAMERA_MODE_SIGNAL no
    // longer gates capture (it now controls only whether the LCD
    // shows the camera preview vs the avatar — see render_task).
    if let Err(e) = sensor.set_streaming(true).await {
        defmt::error!(
            "camera: set_streaming(true) failed ({:?}); camera disabled",
            defmt::Debug2Format(&e),
        );
        park_forever().await;
    }
    defmt::info!(
        "camera: GC0308 SCCB init complete (chip ID 0x{=u8:02x}) — streaming ON for tracking",
        gc0308::CHIP_ID
    );

    // Build the LCD_CAM camera in slave mode. CoreS3 does not route
    // XCLK to a GPIO — the sensor has its own oscillator — so we omit
    // `with_master_clock(...)`. PCLK / HREF / VSYNC + 8 data lines are
    // configured exhaustively below.
    let cam_config = CamConfig::default()
        .with_frequency(Rate::from_hz(PIXEL_CLOCK_HZ))
        .with_byte_order(ByteOrder::default())
        .with_eof_mode(EofMode::VsyncSignal);

    let lcd_cam = LcdCam::new(p.lcd_cam);
    let camera = match Camera::new(lcd_cam.cam, p.dma, cam_config) {
        Ok(cam) => cam
            .with_pixel_clock(p.pclk)
            .with_h_enable(p.href)
            .with_vsync(p.vsync)
            .with_data0(p.d0)
            .with_data1(p.d1)
            .with_data2(p.d2)
            .with_data3(p.d3)
            .with_data4(p.d4)
            .with_data5(p.d5)
            .with_data6(p.d6)
            .with_data7(p.d7),
        Err(e) => {
            defmt::error!(
                "camera: LCD_CAM config rejected ({:?}); camera disabled",
                defmt::Debug2Format(&e),
            );
            park_forever().await;
        }
    };

    // Ping-pong DMA frame buffers: data lives in PSRAM via `Box::leak`,
    // descriptors live in internal SRAM via `StaticCell` so the GDMA
    // engine can chase them at full speed (PSRAM access is too slow
    // for the descriptor walk).
    let Ok((buf_a, buf_b)) = alloc_ping_pong() else {
        defmt::error!("camera: PSRAM allocation for DMA ping-pong buffers failed; camera disabled");
        park_forever().await;
    };
    // Two scratch slots for cross-task hand-off. The camera task copies
    // each just-filled DMA buffer here (alternating slot per ping-pong
    // index); the render task reads via the `&'static [u8]` slice
    // published on `CAMERA_FRAME_SIGNAL`.
    //
    // Held as raw `*mut u8` pointers because Rust's borrow checker
    // can't express the alternating-mut-access pattern across two
    // tasks without a runtime mutex. The underlying memory is leaked
    // PSRAM (always valid for `'static`) and the ping-pong invariant
    // (camera writes slot N while render reads slot N-1) is enforced
    // by `active_idx`, not the borrow checker.
    let (scratch_ptrs, scratch_len) = alloc_scratch_slots();
    defmt::info!(
        "camera: PSRAM buffers allocated — DMA: {} KiB × {}, scratch: {} KiB × {}",
        FRAME_BYTES / 1024,
        BUFFER_COUNT,
        FRAME_BYTES / 1024,
        BUFFER_COUNT,
    );

    let mut camera = camera;
    let mut buf_slot_a: Option<DmaRxBuf> = Some(buf_a);
    let mut buf_slot_b: Option<DmaRxBuf> = Some(buf_b);
    let mut active_idx: usize = 0;

    // Tracker: block-grid motion analysis on each captured frame.
    // Defaults are tuned in the `tracker` crate; tweak via
    // `TrackerConfig` here if on-device tuning calls for it.
    let mut tracker = Tracker::new(TrackerConfig::DEFAULT);
    let mut last_step_at = Instant::now();

    loop {
        // Pick the buffer to fill on this iteration; the OTHER one
        // holds the most-recently-completed frame the render task is
        // (probably) reading. We use `Option::take()` so the slot
        // briefly holds `None` while the buffer lives inside the
        // `CameraTransfer` — the next iteration finds it `Some` again
        // either via `transfer.wait()` returning the buffer (success
        // path) or via the early-return error branch.
        let next_buf = if active_idx == 0 {
            buf_slot_a.take()
        } else {
            buf_slot_b.take()
        };
        let Some(next_buf) = next_buf else {
            defmt::error!("camera: ping-pong slot was unexpectedly empty; resyncing");
            Timer::after(Duration::from_millis(20)).await;
            continue;
        };

        let transfer = match camera.receive(next_buf) {
            Ok(t) => t,
            Err((e, cam, returned)) => {
                defmt::warn!(
                    "camera: DMA receive setup failed ({:?}); resyncing",
                    defmt::Debug2Format(&e),
                );
                camera = cam;
                if active_idx == 0 {
                    buf_slot_a = Some(returned);
                } else {
                    buf_slot_b = Some(returned);
                }
                Timer::after(Duration::from_millis(20)).await;
                continue;
            }
        };

        // Wait for the DMA to fill the buffer. CAM_STOP_EN + EofMode::
        // VsyncSignal makes the peripheral stop after one frame, so
        // is_done() flips true at the VSYNC edge that ends this frame.
        // The wait loop yields to embassy so other tasks (audio,
        // touch, sensors) keep running.
        loop {
            if transfer.is_done() {
                break;
            }
            // 1 ms granularity is well below the ~33 ms inter-frame
            // period and well above the embassy timer tick — accurate
            // enough to catch the EOF without burning CPU.
            Timer::after(Duration::from_millis(1)).await;
        }

        let (result, returned_camera, completed_buf) = transfer.wait();
        camera = returned_camera;
        if let Err(e) = result {
            defmt::warn!(
                "camera: DMA wait error ({:?}) — discarding frame",
                defmt::Debug2Format(&e),
            );
        } else {
            // Copy the DMA buffer into the matching scratch slot and
            // publish a `&'static [u8]` view to the render task.
            //
            // SAFETY: `scratch_ptrs[active_idx]` is a leaked-PSRAM
            // pointer valid for `scratch_len` bytes for the lifetime
            // of the binary. The ping-pong invariant ensures the
            // render task only reads the slot whose `active_idx` was
            // most recently signalled — the slot the camera task is
            // currently writing has not yet been signalled, so no
            // reader has a live `&[u8]` into it.
            let dst_ptr = scratch_ptrs[active_idx];
            let src = completed_buf.as_slice();
            let copy_len = src.len().min(scratch_len);
            unsafe {
                core::ptr::copy_nonoverlapping(src.as_ptr(), dst_ptr, copy_len);
            }
            let view: &'static [u8] = unsafe { core::slice::from_raw_parts(dst_ptr, copy_len) };
            CAMERA_FRAME_SIGNAL.signal(view);

            // Run the block-grid tracker on this frame and publish
            // the observation. `dt_ms` is the wall-clock interval
            // since the previous step — feeds the tracker's idle /
            // return-to-centre logic.
            let now = Instant::now();
            let dt_ms =
                u32::try_from(now.duration_since(last_step_at).as_millis()).unwrap_or(u32::MAX);
            last_step_at = now;
            let outcome = tracker.step(view, dt_ms);
            CAMERA_TRACKING_SIGNAL.signal(TrackingObservation {
                target_pose: outcome.target,
                fired_cells: outcome.fired_cells,
                motion: motion_to_engine(outcome.motion),
            });

            // Honour any pending capture request — log frame stats +
            // a 32×24 RGB565 thumbnail strip over RTT.
            if CAMERA_CAPTURE_REQUEST.try_take().is_some() {
                log_capture_stats(active_idx, view);
            }
        }

        // Park the just-filled buffer back in the slot we took it from
        // and flip to the other slot for the next iteration.
        if active_idx == 0 {
            buf_slot_a = Some(completed_buf);
        } else {
            buf_slot_b = Some(completed_buf);
        }
        active_idx = (active_idx + 1) % BUFFER_COUNT;
    }
}

/// Allocate the two ping-pong DMA buffers + their descriptor tables.
///
/// Buffers go in PSRAM via `Box::leak` (300 KiB total — fits comfortably
/// alongside the 150 KiB avatar framebuffer in 8 MiB PSRAM).
/// Descriptors live in `StaticCell`-backed arrays in internal SRAM so
/// the GDMA engine can walk them without paying PSRAM read latency on
/// every chunk.
fn alloc_ping_pong() -> Result<(DmaRxBuf, DmaRxBuf), ()> {
    use alloc::vec;
    use esp_hal::dma::DmaDescriptor;
    use static_cell::StaticCell;

    static DESCS_A: StaticCell<[DmaDescriptor; DESCRIPTORS_PER_BUFFER]> = StaticCell::new();
    static DESCS_B: StaticCell<[DmaDescriptor; DESCRIPTORS_PER_BUFFER]> = StaticCell::new();

    let descs_a = DESCS_A.init([DmaDescriptor::EMPTY; DESCRIPTORS_PER_BUFFER]);
    let descs_b = DESCS_B.init([DmaDescriptor::EMPTY; DESCRIPTORS_PER_BUFFER]);

    let buf_a_vec: alloc::vec::Vec<u8> = vec![0u8; FRAME_BYTES];
    let buf_second_vec: alloc::vec::Vec<u8> = vec![0u8; FRAME_BYTES];
    let pingpong_a: &'static mut [u8] = alloc::boxed::Box::leak(buf_a_vec.into_boxed_slice());
    let pingpong_b: &'static mut [u8] = alloc::boxed::Box::leak(buf_second_vec.into_boxed_slice());

    let buf_a = DmaRxBuf::new(descs_a, pingpong_a).map_err(|_| ())?;
    let buf_b = DmaRxBuf::new(descs_b, pingpong_b).map_err(|_| ())?;
    Ok((buf_a, buf_b))
}

/// Allocate the two cross-task scratch slots and return raw pointers
/// + length.
///
/// Each slot is a leaked PSRAM-backed `Box<[u8]>` of [`FRAME_BYTES`].
/// We return `*mut u8` pointers so the camera task can hand-off
/// immutable views (`&'static [u8]`) to the render task without
/// running into the borrow checker's "can't have `&mut` and `&` at
/// the same time" rule. The hand-off is sound because of the
/// ping-pong alternation invariant — see the camera-task SAFETY
/// comment at the publish site.
///
/// Infallible — `alloc::vec!` panics on OOM, so by reaching the
/// return statement we know the allocation succeeded.
fn alloc_scratch_slots() -> ([*mut u8; BUFFER_COUNT], usize) {
    use alloc::vec;

    let v0: alloc::vec::Vec<u8> = vec![0u8; FRAME_BYTES];
    let v1: alloc::vec::Vec<u8> = vec![0u8; FRAME_BYTES];
    let s0: &'static mut [u8] = alloc::boxed::Box::leak(v0.into_boxed_slice());
    let s1: &'static mut [u8] = alloc::boxed::Box::leak(v1.into_boxed_slice());
    let len = s0.len();
    debug_assert_eq!(len, s1.len());
    ([s0.as_mut_ptr(), s1.as_mut_ptr()], len)
}

/// Log capture-frame stats + a 32×24 RGB565 thumbnail over RTT.
///
/// Stats: mean luminance (rough proxy from RGB565 channels), min/max
/// pixel values, frame index. Thumbnail: nearest-neighbour decimation
/// of 320×240 → 32×24. Total RTT cost ~1.5 KiB hex-dumped — fits
/// comfortably in a defmt frame.
#[allow(
    clippy::cast_possible_truncation,
    reason = "luminance bytes are bounded to 0..=255 by construction (Rec. 601 \
              weights sum to 100 and per-channel maxima are 255), so the u32→u8 \
              casts cannot truncate"
)]
fn log_capture_stats(buffer_idx: usize, frame: &[u8]) {
    /// Thumbnail width — 32 columns × 24 rows = 768 pixels = 1.5 KiB.
    const THUMB_W: usize = 32;
    /// Thumbnail height — paired with [`THUMB_W`].
    const THUMB_H: usize = 24;

    if frame.len() < FRAME_BYTES {
        defmt::warn!(
            "camera: capture frame undersized ({} < {})",
            frame.len(),
            FRAME_BYTES,
        );
        return;
    }

    // Stats: parse RGB565 (big-endian on this peripheral) and compute
    // a coarse luminance estimate. We sample every 4th pixel to keep
    // the math cheap while still reflecting real frame content.
    let mut sum_lum: u32 = 0;
    let mut min_lum: u8 = u8::MAX;
    let mut max_lum: u8 = 0;
    let mut samples: u32 = 0;
    for chunk in frame.chunks_exact(8) {
        // Parse one RGB565 pixel (big-endian). The peripheral's
        // ByteOrder default emits high byte first.
        let pixel = (u16::from(chunk[0]) << 8) | u16::from(chunk[1]);
        let r = u32::from((pixel >> 11) & 0x1F) * 255 / 31;
        let g = u32::from((pixel >> 5) & 0x3F) * 255 / 63;
        let b = u32::from(pixel & 0x1F) * 255 / 31;
        // Rec. 601 luma approx: 0.30 R + 0.59 G + 0.11 B
        let lum = ((30 * r + 59 * g + 11 * b) / 100) as u8;
        sum_lum += u32::from(lum);
        if lum < min_lum {
            min_lum = lum;
        }
        if lum > max_lum {
            max_lum = lum;
        }
        samples += 1;
    }
    let mean_lum = sum_lum.checked_div(samples).unwrap_or(0) as u8;

    defmt::info!(
        "camera: capture[{}] mean_lum={=u8} min={=u8} max={=u8} samples={=u32}",
        buffer_idx,
        mean_lum,
        min_lum,
        max_lum,
        samples,
    );

    // Step: width / 32 = 10 pixels horizontally; height / 24 = 10 rows.
    let stride_x = (FRAME_WIDTH as usize) / THUMB_W;
    let stride_y = (FRAME_HEIGHT as usize) / THUMB_H;
    let mut thumb = [0u16; THUMB_W * THUMB_H];
    for ty in 0..THUMB_H {
        for tx in 0..THUMB_W {
            let src_x = tx * stride_x;
            let src_y = ty * stride_y;
            let byte_idx = (src_y * (FRAME_WIDTH as usize) + src_x) * BYTES_PER_PIXEL;
            let pixel = (u16::from(frame[byte_idx]) << 8) | u16::from(frame[byte_idx + 1]);
            thumb[ty * THUMB_W + tx] = pixel;
        }
    }
    defmt::info!("camera: thumbnail (RGB565, 32x24): {=[?]}", thumb);
}

/// Park the task forever. Used when init fails irrecoverably so the
/// rest of the firmware (avatar, audio, etc.) keeps running without
/// the camera. The `60 s` interval is arbitrary — the task is parked,
/// not polling.
async fn park_forever() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
