//! Headless simulator for `stackchan-core`.
//!
//! Test-oriented utilities:
//!
//! - [`FakeClock`]: a deterministic [`Clock`] whose time is set by tests.
//! - [`Framebuffer`]: a `Vec<Rgb565>`-backed [`DrawTarget`] that lets render
//!   regression tests assert on the output of `Entity::draw` without running
//!   on hardware.
//! - [`RecordingHead`]: a [`HeadDriver`] impl that captures the
//!   `(Instant, Pose)` trajectory, for golden tests of motion modifiers.
//! - [`TrackingScenario`]: a builder that produces a sequence of
//!   [`TrackingObservation`] values across simulated time, for tests of
//!   the camera-tracker → cognition handoff.
//!
//! [`Modifier`]: stackchan_core::Modifier
//! [`DrawTarget`]: embedded_graphics::draw_target::DrawTarget
//! [`TrackingObservation`]: stackchan_core::TrackingObservation

#![deny(unsafe_code)]

use core::cell::Cell;
use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::{Rgb565, RgbColor},
};
use stackchan_core::{Clock, HeadDriver, Instant, Pose, TrackingMotion, TrackingObservation};

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
/// any call to `Entity::draw` that typechecks against a
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
/// bus: drive the modifier pipeline, push `entity.motor.head_pose` into a
/// `RecordingHead` each tick, then assert amplitude / period / trajectory
/// bounds on [`RecordingHead::records`].
///
/// The [`HeadDriver`] impl is `async` to match the firmware's `SCServo`
/// driver shape, but the recorded future is always immediately `Ready` —
/// tests can drive it with the small `block_on` helper in the
/// `head_sway.rs` integration test, or skip the trait entirely and inspect
/// `entity.motor.head_pose` directly for simple cases.
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

/// Replayable sequence of [`TrackingObservation`] values for sim tests
/// of the camera-tracker → cognition handoff.
///
/// A scenario is built as a chain of "blocks" — each block is a
/// duration during which a single observation (or the absence of one,
/// modelling a drain miss) is published every tick. [`Self::iter`]
/// walks the chain and yields one `(Instant, Option<TrackingObservation>)`
/// per tick, ready for tests to write into `entity.perception.tracking`
/// before each `Director::run`.
///
/// ## Tick semantics
///
/// Tick cadence is fixed at construction (default `33` ms ≈ the
/// firmware tracker's ~30 Hz rate). Block durations are floored to the
/// cadence: a `duration_ms` block produces `floor(duration_ms / tick_ms)`
/// ticks at offsets `0, tick_ms, 2*tick_ms, …, (count-1)*tick_ms`.
///
/// In particular:
///
/// - The **first** tick of a block sits at the block's start offset.
/// - The **last** tick sits at `start + (count - 1) * tick_ms` —
///   *not* at `start + duration_ms`.
/// - The **next block** starts at `start + count * tick_ms` (any
///   sub-tick remainder of the previous block is dropped).
///
/// To size a block to produce exactly `N` ticks, use
/// [`Self::duration_for_ticks`] — it returns `N * tick_ms` and avoids
/// the off-by-one trap of naïve `N * 33`-style arithmetic when the
/// cadence isn't `33` ms.
///
/// ## Example
///
/// ```no_run
/// use stackchan_sim::TrackingScenario;
/// use stackchan_core::Pose;
///
/// let scenario = TrackingScenario::new(33)
///     .silent(500)
///     .tracking(Pose::new(10.0, 5.0), 1_000)
///     .with_face((0.5, 0.0))
///     .holding(Pose::new(10.0, 5.0), 500)
///     .silent(2_000);
/// for (now, obs) in scenario.iter() {
///     // entity.tick.now = now;
///     // entity.perception.tracking = obs;
///     // director.run(&mut entity, now);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TrackingScenario {
    /// Per-tick advance, in ms.
    tick_ms: u64,
    /// Time-ordered list of observation blocks.
    blocks: Vec<Block>,
}

/// One contiguous span of identical observations within a [`TrackingScenario`].
#[derive(Debug, Clone)]
struct Block {
    /// Block length in ms. Floored to `tick_ms` granularity by [`TrackingScenario::iter`].
    duration_ms: u64,
    /// Observation published every tick of this block. `None` models a
    /// firmware drain miss — `entity.perception.tracking` stays at the
    /// previous value (or `None` if never set).
    template: Option<TrackingObservation>,
}

impl TrackingScenario {
    /// Construct an empty scenario with the given per-tick cadence.
    /// `33` ms matches the firmware tracker's ~30 Hz publish rate.
    ///
    /// # Panics
    ///
    /// Panics if `tick_ms` is `0` — that would produce an infinite
    /// observation stream per block.
    #[must_use]
    pub fn new(tick_ms: u64) -> Self {
        assert!(tick_ms > 0, "tick_ms must be > 0");
        Self {
            tick_ms,
            blocks: Vec::new(),
        }
    }

    /// Append a block where `perception.tracking` stays `None` for
    /// `duration_ms` — the firmware drain hasn't published an
    /// observation this tick (e.g. boot warmup, brief drain miss).
    #[must_use]
    pub fn silent(mut self, duration_ms: u64) -> Self {
        self.blocks.push(Block {
            duration_ms,
            template: None,
        });
        self
    }

    /// Append `duration_ms` of [`TrackingMotion::Tracking`] observations
    /// pointing at `target`. No face component (`face_present = false`).
    #[must_use]
    pub fn tracking(mut self, target: Pose, duration_ms: u64) -> Self {
        self.blocks.push(Block {
            duration_ms,
            template: Some(observation(TrackingMotion::Tracking, target)),
        });
        self
    }

    /// Attach `face_present = true` and the supplied normalised
    /// `centroid` in `[-1, 1]` to the most recently appended block.
    /// Drives engagement-side cognition for that block. Composes with
    /// any motion-class block, so face data can ride a `tracking`,
    /// `holding`, or `returning` block — matching the firmware shape
    /// where face detection is decoupled from motion class.
    ///
    /// # Panics
    ///
    /// Panics if no block has been appended yet, or if the most
    /// recent block is `silent` — drain-miss ticks have no
    /// observation to mutate.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "test-helper API: misuse (no prior block / silent block) is a \
                  programming error in a test, not a runtime condition; the panic \
                  message names the fix"
    )]
    pub fn with_face(mut self, centroid: (f32, f32)) -> Self {
        let block = self
            .blocks
            .last_mut()
            .expect("with_face called before any observation block was appended");
        let obs = block.template.as_mut().expect(
            "with_face cannot attach to a `silent` block; \
             call `.tracking()`, `.holding()`, or `.returning()` first",
        );
        obs.face_present = true;
        obs.face_centroid = Some(centroid);
        self
    }

    /// Append `duration_ms` of [`TrackingMotion::Holding`] observations
    /// — same target each tick, no fresh motion. Tracker still believes
    /// the target is meaningful but isn't seeing change frames.
    #[must_use]
    pub fn holding(mut self, target: Pose, duration_ms: u64) -> Self {
        self.blocks.push(Block {
            duration_ms,
            template: Some(observation(TrackingMotion::Holding, target)),
        });
        self
    }

    /// Append `duration_ms` of [`TrackingMotion::Returning`] observations
    /// — tracker is slewing back to neutral after an idle timeout.
    #[must_use]
    pub fn returning(mut self, duration_ms: u64) -> Self {
        self.blocks.push(Block {
            duration_ms,
            template: Some(observation(TrackingMotion::Returning, Pose::NEUTRAL)),
        });
        self
    }

    /// Iterate `(Instant, Option<TrackingObservation>)` for each tick
    /// in the scenario, in time order. The first tick is at
    /// `Instant::ZERO`; each subsequent tick advances by `tick_ms`.
    pub fn iter(&self) -> impl Iterator<Item = (Instant, Option<TrackingObservation>)> + '_ {
        let tick_ms = self.tick_ms;
        // `scan` carries the running ms offset across blocks; the
        // inner `flat_map` materialises the per-tick (now, obs)
        // pairs. Templates clone cheaply (the `candidates` heapless
        // vec is empty in scenarios constructed via the public API).
        self.blocks
            .iter()
            .scan(0_u64, move |t_ms, block| {
                let count = block.duration_ms / tick_ms;
                let start_ms = *t_ms;
                *t_ms = start_ms.saturating_add(count.saturating_mul(tick_ms));
                Some((start_ms, count, block.template.clone()))
            })
            .flat_map(move |(start_ms, count, template)| {
                (0..count).map(move |i| {
                    let now = Instant::from_millis(start_ms + i * tick_ms);
                    (now, template.clone())
                })
            })
    }

    /// Per-tick advance, in ms. Useful for tests that want to drive
    /// auxiliary state alongside the scenario at the same cadence.
    #[must_use]
    pub const fn tick_ms(&self) -> u64 {
        self.tick_ms
    }

    /// Block duration, in ms, that produces exactly `ticks` iterations
    /// at the configured cadence. Use in test setup instead of
    /// `N * 33`-style literals so a future cadence change can't
    /// silently shift tick counts via the floor in [`Self::iter`].
    #[must_use]
    pub const fn duration_for_ticks(&self, ticks: u64) -> u64 {
        ticks * self.tick_ms
    }
}

/// Drive an always-Ready future to completion synchronously.
///
/// Useful for [`RecordingHead::set_pose`], which returns a future whose
/// impl is a single immediately-Ready value but typechecks as `async`.
/// Spins in a tight loop (does NOT yield) if the future ever returns
/// `Pending`, surfacing misuse — `RecordingHead` is the intended caller.
pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
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

/// Build a baseline [`TrackingObservation`] for the given motion class.
///
/// Mirrors the helper inside `attention_from_tracking::tests` so sim
/// tests can construct observations without re-deriving the `fired_cells`
/// convention. `Tracking` reports a non-zero `fired_cells` (matches the
/// real tracker, which emits Tracking only when at least one grid cell
/// fired); other motion classes report `0`.
const fn observation(motion: TrackingMotion, target: Pose) -> TrackingObservation {
    TrackingObservation {
        target_pose: target,
        fired_cells: if matches!(motion, TrackingMotion::Tracking) {
            4
        } else {
            0
        },
        motion,
        candidates: heapless::Vec::new(),
        face_present: false,
        face_centroid: None,
    }
}

#[cfg(test)]
#[allow(
    clippy::field_reassign_with_default,
    reason = "test setup reads better as `let mut a = Entity::default(); a.emotion = …;` than the struct-update equivalent"
)]
#[allow(
    clippy::unwrap_used,
    reason = "test literals are compile-time non-zero; the unwrap can't fire"
)]
mod integration_tests {
    use super::*;
    use stackchan_core::modifiers::{Blink, Breath, EmotionCycle, IdleDrift, StyleFromEmotion};
    use stackchan_core::{Director, Emotion, Entity, EyePhase, Modifier, SCALE_DEFAULT};

    /// End-to-end: drive a Blink + Breath + `IdleDrift` stack via the
    /// `Director` for 60 simulated seconds at 30 FPS and verify the
    /// avatar never enters a nonsensical state (e.g. eyes walking
    /// off-screen, weight out of range). Routing through `Director`
    /// also exercises the debug-mode `writes:` enforcement on every
    /// frame.
    #[test]
    fn sixty_second_composition_is_stable() {
        let clock = FakeClock::new();
        let mut avatar = Entity::default();
        let mut blink = Blink::new();
        let mut breath = Breath::new();
        let mut drift = IdleDrift::with_seed(core::num::NonZeroU32::new(0xDEAD_BEEF).unwrap());
        let mut director = Director::new();
        director.add_modifier(&mut blink).unwrap();
        director.add_modifier(&mut breath).unwrap();
        director.add_modifier(&mut drift).unwrap();

        let tick_ms = 33; // ~30 FPS
        let total_ticks = 60_000 / tick_ms;

        for _ in 0..total_ticks {
            director.run(&mut avatar, clock.now());

            // Invariants that must hold every frame:
            assert!(avatar.face.left_eye.weight <= 100);
            assert!(avatar.face.right_eye.weight <= 100);
            // Framebuffer is 320x240; eyes must stay reasonably on-face.
            assert!(
                (0..320).contains(&avatar.face.left_eye.center.x),
                "left eye x = {}",
                avatar.face.left_eye.center.x
            );
            assert!(
                (0..320).contains(&avatar.face.right_eye.center.x),
                "right eye x = {}",
                avatar.face.right_eye.center.x
            );

            clock.advance(tick_ms);
        }
    }

    /// Two `IdleDrift`s seeded with distinct values must produce
    /// distinct eye-position sequences. This is the host-side guard
    /// for the firmware boot path that samples `esp_hal::rng::Rng`
    /// once per boot — if seed propagation broke (e.g. a future
    /// refactor of `IdleDrift::with_seed` quietly ignored its arg),
    /// every boot would produce identical drift sequences and this
    /// test would fail.
    #[test]
    fn distinct_seeds_produce_distinct_drift_sequences() {
        // 7 ticks × `DEFAULT_INTERVAL_MS` (4 s) = 7 drift events.
        // Two unrelated seeds — using compile-time non-zero literals
        // sidesteps the test-only `unwrap` surface.
        let seed_a = core::num::NonZeroU32::new(0x1234_5678).unwrap();
        let seed_b = core::num::NonZeroU32::new(0xCAFE_BABE).unwrap();
        let mut drift_a = IdleDrift::with_seed(seed_a);
        let mut drift_b = IdleDrift::with_seed(seed_b);

        let mut avatar_a = Entity::default();
        let mut avatar_b = Entity::default();
        let clock = FakeClock::new();

        let interval_ms = 4_000_u64; // matches IdleDrift::DEFAULT_INTERVAL_MS
        let mut diverged = false;
        for i in 0..7 {
            // Tick exactly at each drift boundary so an offset is
            // applied on every iteration after the scheduling tick.
            let now = stackchan_core::Instant::from_millis(i * interval_ms);
            avatar_a.tick.now = now;
            drift_a.update(&mut avatar_a);
            avatar_b.tick.now = now;
            drift_b.update(&mut avatar_b);
            if avatar_a.face.left_eye.center != avatar_b.face.left_eye.center {
                diverged = true;
                break;
            }
        }
        // Use clock to keep `FakeClock` import consistent with the
        // surrounding tests; advancing here is a no-op but keeps the
        // test shape uniform if a future Modifier consults the clock
        // outside its `update` arg.
        clock.advance(interval_ms * 7);

        assert!(
            diverged,
            "two distinct IdleDrift seeds produced identical eye sequences over 7 drift ticks"
        );
    }

    #[test]
    fn blink_frequency_over_one_minute() {
        let clock = FakeClock::new();
        let mut avatar = Entity::default();
        let mut blink = Blink::new();

        let tick_ms = 10;
        let total_ticks = 60_000 / tick_ms;

        let mut blink_count = 0_u32;
        let mut prev_phase = EyePhase::Open;

        for _ in 0..total_ticks {
            avatar.tick.now = clock.now();
            blink.update(&mut avatar);
            if avatar.face.left_eye.phase == EyePhase::Closed && prev_phase == EyePhase::Open {
                blink_count += 1;
            }
            prev_phase = avatar.face.left_eye.phase;
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
    /// blink → breath → drift) via the `Director` for one complete
    /// cycle of the default emotion rotation and assert that every
    /// emotion visibly propagates into the style fields. This is the
    /// host-side mirror of what the CoreS3 render task runs at 30 FPS,
    /// including the debug-mode `writes:` enforcement.
    #[test]
    fn full_stack_cycles_through_every_default_emotion() {
        let clock = FakeClock::new();
        let mut avatar = Entity::default();
        let mut cycle = EmotionCycle::new();
        let mut style = StyleFromEmotion::new();
        let mut blink = Blink::new();
        let mut breath = Breath::new();
        let mut drift = IdleDrift::with_seed(core::num::NonZeroU32::new(0xDEAD_BEEF).unwrap());
        let mut director = Director::new();
        director.add_modifier(&mut cycle).unwrap();
        director.add_modifier(&mut style).unwrap();
        director.add_modifier(&mut blink).unwrap();
        director.add_modifier(&mut breath).unwrap();
        director.add_modifier(&mut drift).unwrap();

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
            director.run(&mut avatar, clock.now());

            // Every frame still satisfies the baseline invariants.
            assert!(avatar.face.left_eye.weight <= 100);
            assert!(avatar.face.right_eye.weight <= 100);

            match avatar.mind.affect.emotion {
                Emotion::Happy if avatar.face.style.cheek_blush > 0 => seen_happy_cheeks = true,
                Emotion::Sad if avatar.face.style.mouth_curve < 0 => seen_sad_frown = true,
                Emotion::Sleepy if avatar.face.left_eye.open_weight < 100 => {
                    seen_sleepy_droop = true;
                }
                Emotion::Surprised if avatar.face.style.eye_scale > 128 => {
                    seen_surprised_wide = true;
                }
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

    /// Regression test for the `StyleFromEmotion → Blink` ordering contract.
    /// The Director sorts modifiers by `(phase, priority,
    /// registration_order)`; `StyleFromEmotion` has priority `-10` while
    /// `Blink` has priority `0`, both in `Phase::Expression`, so
    /// `StyleFromEmotion` runs first regardless of registration order.
    /// Blink's effect on `Eye::weight` therefore reflects the *current*
    /// tick's `blink_rate_scale`, not a stale value from a previous tick.
    /// This pin would catch a future priority swap that broke the
    /// invariant.
    #[test]
    fn canonical_order_propagates_blink_rate_within_one_tick() {
        let mut avatar = Entity::default();
        avatar.mind.affect.emotion = Emotion::Surprised;
        let mut style = StyleFromEmotion::new();
        let mut blink = Blink::new();
        let mut director = Director::new();
        // Register in REVERSE of canonical order to prove the Director's
        // sort by `priority` is what enforces ordering, not insertion.
        director.add_modifier(&mut blink).unwrap();
        director.add_modifier(&mut style).unwrap();

        // First frame: Surprised establishes from + to in StyleFromEmotion,
        // rate = 0 takes effect, Blink suppresses on the same frame.
        director.run(&mut avatar, Instant::from_millis(0));
        assert_eq!(
            avatar.face.style.blink_rate_scale, 0,
            "StyleFromEmotion must snap to target on first observation of a new emotion"
        );
        assert_eq!(
            avatar.face.left_eye.phase,
            EyePhase::Open,
            "rate == 0 must force eyes open on the same frame StyleFromEmotion wrote it"
        );

        // Baseline sanity: switch to Neutral, advance past the
        // 300 ms transition window, rate ramps to SCALE_DEFAULT.
        avatar.mind.affect.emotion = Emotion::Neutral;
        director.run(&mut avatar, Instant::from_millis(10_000));
        director.run(&mut avatar, Instant::from_millis(10_400));
        assert_eq!(avatar.face.style.blink_rate_scale, SCALE_DEFAULT);
    }
}
