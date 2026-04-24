//! Headless simulator for `stackchan-core`.
//!
//! Test-oriented utilities:
//!
//! - [`FakeClock`]: a deterministic [`Clock`] whose time is set by tests.
//! - [`Framebuffer`]: a `Vec<Rgb565>`-backed [`DrawTarget`] that lets render
//!   regression tests assert on the output of `Avatar::draw` without running
//!   on hardware.
//! - [`RecordingHead`]: a [`HeadDriver`] impl that captures the
//!   `(Instant, Pose)` trajectory, for golden tests of motion modifiers.
//!
//! [`Modifier`]: stackchan_core::Modifier
//! [`DrawTarget`]: embedded_graphics::draw_target::DrawTarget

#![deny(unsafe_code)]

use core::cell::Cell;
use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::{Rgb565, RgbColor},
};
use stackchan_core::{Clock, HeadDriver, Instant, Pose};

/// A [`Clock`] whose current time is set explicitly by tests.
///
/// Unlike a wall-clock source, `FakeClock` never drifts and never advances
/// on its own. Tests call [`FakeClock::advance`] or
/// [`FakeClock::set`] between assertions.
#[derive(Debug, Default)]
pub struct FakeClock {
    /// Current time; uses `Cell` so `Clock::now` can take `&self`.
    now: Cell<Instant>,
}

impl FakeClock {
    /// Construct a new `FakeClock` at `Instant::ZERO`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            now: Cell::new(Instant::ZERO),
        }
    }

    /// Advance the clock by `delta_ms` milliseconds.
    pub fn advance(&self, delta_ms: u64) {
        self.now.set(self.now.get() + delta_ms);
    }

    /// Set the clock to an absolute instant. Callers are responsible for
    /// monotonicity; this is a test helper that trusts the test author.
    pub fn set(&self, to: Instant) {
        self.now.set(to);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
}

/// An in-memory RGB565 framebuffer used for render-regression tests.
///
/// Implements [`DrawTarget`] with [`core::convert::Infallible`] errors, so
/// any call to `Avatar::draw` that typechecks against a
/// `DrawTarget<Color = Rgb565>` also runs against this buffer. Pixels
/// outside the buffer bounds are silently dropped, matching how
/// `embedded-graphics` clips to [`OriginDimensions`].
pub struct Framebuffer {
    /// Row-major RGB565 pixel buffer of length `width * height`.
    pixels: Vec<Rgb565>,
    /// Framebuffer width in pixels.
    width: u32,
    /// Framebuffer height in pixels.
    height: u32,
}

impl Framebuffer {
    /// Create a `width x height` framebuffer filled with black.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        // try_from is lossless on 32/64-bit hosts; saturate on 16-bit
        // (which we never build for) to avoid an `as usize` cast.
        let w = usize::try_from(width).unwrap_or(0);
        let h = usize::try_from(height).unwrap_or(0);
        let len = w.saturating_mul(h);
        Self {
            pixels: vec![Rgb565::BLACK; len],
            width,
            height,
        }
    }

    /// Read the pixel at `(x, y)`. Returns `None` if the coordinate is
    /// outside the buffer.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgb565> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = usize::try_from(y.saturating_mul(self.width).saturating_add(x)).ok()?;
        self.pixels.get(idx).copied()
    }

    /// Borrow the underlying pixel slice (row-major, `width * height` long).
    #[must_use]
    pub fn as_slice(&self) -> &[Rgb565] {
        &self.pixels
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let Ok(x) = u32::try_from(point.x) else {
                continue;
            };
            let Ok(y) = u32::try_from(point.y) else {
                continue;
            };
            if x >= self.width || y >= self.height {
                continue;
            }
            let Ok(idx) = usize::try_from(y.saturating_mul(self.width).saturating_add(x)) else {
                continue;
            };
            if let Some(cell) = self.pixels.get_mut(idx) {
                *cell = color;
            }
        }
        Ok(())
    }
}

/// [`HeadDriver`] that records every `set_pose` call into a `Vec`.
///
/// Pair with [`FakeClock`] to test motion modifiers without a real `SCServo`
/// bus: drive the modifier pipeline, push `avatar.head_pose` into a
/// `RecordingHead` each tick, then assert amplitude / period / trajectory
/// bounds on [`RecordingHead::records`].
///
/// The [`HeadDriver`] impl is `async` to match the firmware's `SCServo`
/// driver shape, but the recorded future is always immediately `Ready` —
/// tests can drive it with the small `block_on` helper in the
/// `head_sway.rs` integration test, or skip the trait entirely and inspect
/// `avatar.head_pose` directly for simple cases.
#[derive(Debug, Default)]
pub struct RecordingHead {
    /// Every `(now, pose)` pair passed to `set_pose`, in call order.
    records: Vec<(Instant, Pose)>,
}

impl RecordingHead {
    /// Construct an empty recorder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// All recorded `(Instant, Pose)` pairs.
    #[must_use]
    pub fn records(&self) -> &[(Instant, Pose)] {
        &self.records
    }

    /// Discard all recorded calls.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl HeadDriver for RecordingHead {
    type Error = core::convert::Infallible;

    async fn set_pose(&mut self, pose: Pose, now: Instant) -> Result<(), Self::Error> {
        self.records.push((now, pose));
        Ok(())
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use stackchan_core::modifiers::{Blink, Breath, EmotionCycle, EmotionStyle, IdleDrift};
    use stackchan_core::{Avatar, Emotion, EyePhase, Modifier};

    /// End-to-end: drive a Blink + Breath + `IdleDrift` stack for 60 simulated
    /// seconds at 30 FPS and verify the avatar never enters a nonsensical
    /// state (e.g. eyes walking off-screen, weight out of range).
    #[test]
    fn sixty_second_composition_is_stable() {
        let clock = FakeClock::new();
        let mut avatar = Avatar::default();
        let mut blink = Blink::new();
        let mut breath = Breath::new();
        let mut drift = IdleDrift::with_seed(0xDEAD_BEEF);

        let tick_ms = 33; // ~30 FPS
        let total_ticks = 60_000 / tick_ms;

        for _ in 0..total_ticks {
            blink.update(&mut avatar, clock.now());
            breath.update(&mut avatar, clock.now());
            drift.update(&mut avatar, clock.now());

            // Invariants that must hold every frame:
            assert!(avatar.left_eye.weight <= 100);
            assert!(avatar.right_eye.weight <= 100);
            // Framebuffer is 320x240; eyes must stay reasonably on-face.
            assert!(
                (0..320).contains(&avatar.left_eye.center.x),
                "left eye x = {}",
                avatar.left_eye.center.x
            );
            assert!(
                (0..320).contains(&avatar.right_eye.center.x),
                "right eye x = {}",
                avatar.right_eye.center.x
            );

            clock.advance(tick_ms);
        }
    }

    #[test]
    fn blink_frequency_over_one_minute() {
        let clock = FakeClock::new();
        let mut avatar = Avatar::default();
        let mut blink = Blink::new();

        let tick_ms = 10;
        let total_ticks = 60_000 / tick_ms;

        let mut blink_count = 0_u32;
        let mut prev_phase = EyePhase::Open;

        for _ in 0..total_ticks {
            blink.update(&mut avatar, clock.now());
            if avatar.left_eye.phase == EyePhase::Closed && prev_phase == EyePhase::Open {
                blink_count += 1;
            }
            prev_phase = avatar.left_eye.phase;
            clock.advance(tick_ms);
        }

        // Default timing is ~5.2s open + 180ms closed = ~11 blinks per minute.
        // Allow wide tolerance for the approximate nature of the state machine.
        assert!(
            (9..=13).contains(&blink_count),
            "expected ~11 blinks in 60s, saw {blink_count}"
        );
    }

    /// Run the full firmware-style modifier stack (emotion cycle → style →
    /// blink → breath → drift) for one complete cycle of the default
    /// emotion rotation and assert that every emotion visibly propagates
    /// into the style fields. This is the host-side mirror of what the
    /// CoreS3 render task runs at 30 FPS.
    #[test]
    fn full_stack_cycles_through_every_default_emotion() {
        let clock = FakeClock::new();
        let mut avatar = Avatar::default();
        let mut cycle = EmotionCycle::new();
        let mut style = EmotionStyle::new();
        let mut blink = Blink::new();
        let mut breath = Breath::new();
        let mut drift = IdleDrift::with_seed(0xDEAD_BEEF);

        // `EmotionCycle::DEFAULT_SEQUENCE` dwell = 4 s × 5 emotions = 20 s.
        // Plus a healthy margin so the last emotion's transition window
        // (300 ms) completes before we assert.
        let tick_ms = 33_u64; // ~30 FPS
        let total_ms = 21_000_u64;
        let ticks = total_ms / tick_ms;

        let mut seen_happy_cheeks = false;
        let mut seen_sad_frown = false;
        let mut seen_sleepy_droop = false;
        let mut seen_surprised_wide = false;

        for _ in 0..ticks {
            cycle.update(&mut avatar, clock.now());
            style.update(&mut avatar, clock.now());
            blink.update(&mut avatar, clock.now());
            breath.update(&mut avatar, clock.now());
            drift.update(&mut avatar, clock.now());

            // Every frame still satisfies the baseline invariants.
            assert!(avatar.left_eye.weight <= 100);
            assert!(avatar.right_eye.weight <= 100);

            match avatar.emotion {
                Emotion::Happy if avatar.cheek_blush > 0 => seen_happy_cheeks = true,
                Emotion::Sad if avatar.mouth_curve < 0 => seen_sad_frown = true,
                Emotion::Sleepy if avatar.left_eye.open_weight < 100 => seen_sleepy_droop = true,
                Emotion::Surprised if avatar.eye_scale > 128 => seen_surprised_wide = true,
                _ => {}
            }

            clock.advance(tick_ms);
        }

        assert!(seen_happy_cheeks, "Happy emotion never raised cheek_blush");
        assert!(
            seen_sad_frown,
            "Sad emotion never produced a frown mouth_curve"
        );
        assert!(
            seen_sleepy_droop,
            "Sleepy emotion never dropped eye.open_weight below 100"
        );
        assert!(
            seen_surprised_wide,
            "Surprised emotion never raised eye_scale above baseline"
        );
    }
}
