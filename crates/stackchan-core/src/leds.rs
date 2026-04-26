//! LED-ring output sink.
//!
//! The entity wears a 12-pixel WS2812 ring; this module maps [`Entity`]
//! state onto a packed [`LedFrame`] of RGB565 pixels that the firmware
//! pushes to the PY32 IO expander.
//!
//! Architectural role: this is the **first** non-[`Modifier`](crate::Modifier) consumer
//! of [`Entity`] state. Modifiers mutate the entity; this is a pure
//! function over it. The firmware's `render_task` runs the
//! [`Director`](crate::Director) stack, then calls [`render_leds`] on
//! the resulting entity and signals the frame out to a transport task.
//! The sim uses the same function against a `FakeClock` to golden-test
//! the mapping without any hardware.
//!
//! ## Mapping
//!
//! - **Emotion → base colour** via a warm/cool palette
//!   ([`Neutral`]=soft white, [`Happy`]=amber, [`Sad`]=deep blue,
//!   [`Sleepy`]=dim violet, [`Surprised`]=cyan).
//! - **Intent overrides emotion colour** for sound-reactive intents:
//!   [`Intent::Listen`] swaps the palette to a teal "I'm listening"
//!   hue regardless of the underlying emotion (which `WakeOnVoice`
//!   typically pins to `Happy`). [`Intent::HearingLoud`] keeps the
//!   `Surprised` cyan but pins brightness to peak — the breath dim
//!   is suppressed so the ring "flashes" for the hold duration.
//! - **Breath-phase → brightness envelope** in `[0.6, 1.0]` over a
//!   6-second triangle cycle, matching the default
//!   [`Breath`](crate::modifiers::Breath) modifier. The LED pulse stays
//!   visually synchronised with the on-screen breath without the LED
//!   pipeline having to introspect the modifier stack's state.
//! - **Global cap at ~40% of full drive** keeps the ring readable
//!   through a head shell without being harsh. Palette values are
//!   stored at full 888 and scaled at render time.
//!
//! The mapping is uniform across all 12 pixels — no per-pixel animation
//! yet. A gaze-direction arc is an obvious next iteration.
//!
//! [`Intent::Listen`]: crate::mind::Intent::Listen
//! [`Intent::HearingLoud`]: crate::mind::Intent::HearingLoud
//!
//! [`Neutral`]: crate::Emotion::Neutral
//! [`Happy`]: crate::Emotion::Happy
//! [`Sad`]: crate::Emotion::Sad
//! [`Sleepy`]: crate::Emotion::Sleepy
//! [`Surprised`]: crate::Emotion::Surprised

use crate::mind::Intent;
use crate::{Emotion, Entity, Instant};

/// Number of pixels on the StackChan LED ring.
pub const LED_COUNT: usize = 12;

/// Peak 8-bit brightness applied to any single channel. WS2812 at full
/// drive is harsh inside a small head shell; ~40% is the sweet spot.
pub const BRIGHTNESS_PEAK: u8 = 102;

/// Fraction of [`BRIGHTNESS_PEAK`] the breath envelope dips to at the
/// exhale minimum. Tenths, so `6` = 60%.
const BREATH_TROUGH_NUM: u32 = 6;
/// Denominator for [`BREATH_TROUGH_NUM`].
const BREATH_TROUGH_DEN: u32 = 10;

/// Full breath cycle in milliseconds. Locked to
/// [`Breath::DEFAULT_CYCLE_MS`](crate::modifiers::Breath) so the LED
/// pulse stays in phase with the on-face breath at the default config.
const BREATH_CYCLE_MS: u64 = 6_000;

/// A full frame of RGB565 pixels, ready to ship to the LED ring.
///
/// Bit layout of each pixel is `RRRRRGGG_GGGBBBBB` (big-endian within
/// the word); the firmware writes the values to the PY32 little-endian
/// on the I²C bus, which is handled by `py32::Py32::write_led_pixels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedFrame(pub [u16; LED_COUNT]);

impl Default for LedFrame {
    /// All pixels off.
    fn default() -> Self {
        Self([0; LED_COUNT])
    }
}

impl LedFrame {
    /// Borrow the underlying pixel array as a slice of `u16`, suitable
    /// for direct hand-off to the PY32 driver's bulk LED write.
    #[must_use]
    pub const fn as_u16_slice(&self) -> &[u16] {
        &self.0
    }
}

/// Emotion → base RGB888 colour palette.
///
/// Packed as `0xRRGGBB`. Conversion to RGB565 happens after brightness
/// scaling so we don't compound quantisation on the way in.
#[must_use]
const fn palette(emotion: Emotion) -> u32 {
    // Exhaustive on purpose: adding an Emotion variant must also extend
    // the palette. `#[non_exhaustive]` only affects downstream matches.
    match emotion {
        Emotion::Neutral => 0x00FF_FFE8,
        Emotion::Happy => 0x00FF_B020,
        Emotion::Sad => 0x0015_30A0,
        Emotion::Sleepy => 0x0030_1448,
        Emotion::Surprised => 0x0030_E0FF,
        Emotion::Angry => 0x00FF_2020,
    }
}

/// "Listening" teal — used as a palette override when
/// [`Intent::Listen`] is held.
///
/// Cool, calm, distinct from both `Happy`'s amber (which `WakeOnVoice`
/// typically pins simultaneously) and `Surprised`'s cyan (used for
/// [`Intent::HearingLoud`]).
const LISTEN_PALETTE: u32 = 0x0010_C0A0;

/// Resolve the per-frame palette: intent overrides emotion for the
/// sound-reactive intents.
#[must_use]
const fn palette_for(intent: Intent, emotion: Emotion) -> u32 {
    match intent {
        Intent::Listen => LISTEN_PALETTE,
        // HearingLoud doesn't override the palette — it relies on the
        // `Surprised` emotion that `StartleOnLoud` writes simultaneously.
        // Idle / BeingPet / PickedUp / Shaken / Tilted / HearingLoud all
        // fall through to the emotion palette.
        Intent::Idle
        | Intent::BeingPet
        | Intent::PickedUp
        | Intent::Shaken
        | Intent::Tilted
        | Intent::HearingLoud => palette(emotion),
    }
}

/// Brightness envelope for the breath-phase pulse at time `now`.
///
/// Returns an 8-bit multiplier where `255` = [`BRIGHTNESS_PEAK`] and
/// the trough value is `BRIGHTNESS_PEAK * BREATH_TROUGH_NUM /
/// BREATH_TROUGH_DEN`. Triangle wave to stay libm-free.
fn breath_brightness(now: Instant) -> u8 {
    let phase = now.as_millis() % BREATH_CYCLE_MS;
    let half = BREATH_CYCLE_MS / 2;
    let within = if phase < half {
        phase
    } else {
        BREATH_CYCLE_MS - phase
    };
    // `within` is in 0..=half (0..=3000); normalise to a 0..=255
    // triangle. The numerator fits in u64 comfortably and the quotient
    // is bounded by 255.
    let t_u64 = within.saturating_mul(255) / half.max(1);
    let t = u32::try_from(t_u64).unwrap_or(255);
    let peak = u32::from(BRIGHTNESS_PEAK);
    let trough = peak * BREATH_TROUGH_NUM / BREATH_TROUGH_DEN;
    let envelope = trough + (peak - trough) * t / 255;
    // envelope <= peak <= BRIGHTNESS_PEAK (102) < 256.
    u8::try_from(envelope).unwrap_or(BRIGHTNESS_PEAK)
}

/// Apply an 8-bit brightness multiplier to one channel.
///
/// `channel * multiplier / 255`, kept in `u32` to avoid overflow on the
/// intermediate product (`255 * 255 = 65_025`).
#[inline]
fn scale_channel(channel: u8, multiplier: u8) -> u8 {
    let scaled = u32::from(channel) * u32::from(multiplier) / 255;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "scaled <= channel <= 255, truncation lossless"
    )]
    {
        scaled as u8
    }
}

/// Pack an RGB888 triple into an RGB565 pixel. Bit layout is
/// `RRRRRGGG_GGGBBBBB` in the returned `u16`.
#[inline]
const fn rgb888_to_565(r: u8, g: u8, b: u8) -> u16 {
    let r5 = (r as u16 & 0xF8) << 8;
    let g6 = (g as u16 & 0xFC) << 3;
    let b5 = (b as u16) >> 3;
    r5 | g6 | b5
}

/// Render the LED ring for the current [`Entity`] state.
///
/// Writes all [`LED_COUNT`] pixels of `out` with a uniform colour
/// derived from `entity.mind.affect.emotion`, dimmed by both the
/// global brightness cap and the current breath-envelope phase at
/// `now`. Deterministic with respect to `(entity.mind.affect.emotion,
/// now)` — host-testable.
pub fn render_leds(entity: &Entity, now: Instant, out: &mut LedFrame) {
    let base = palette_for(entity.mind.intent, entity.mind.affect.emotion);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "bitmasked to 8 bits before truncation"
    )]
    let (r_raw, g_raw, b_raw) = (
        ((base >> 16) & 0xFF) as u8,
        ((base >> 8) & 0xFF) as u8,
        (base & 0xFF) as u8,
    );
    // `HearingLoud` pins brightness to peak — the breath dim is
    // suppressed so the ring reads as a "flash" for the hold
    // duration. Other intents follow the breath envelope.
    let brightness = if matches!(entity.mind.intent, Intent::HearingLoud) {
        BRIGHTNESS_PEAK
    } else {
        breath_brightness(now)
    };
    let pixel = rgb888_to_565(
        scale_channel(r_raw, brightness),
        scale_channel(g_raw, brightness),
        scale_channel(b_raw, brightness),
    );
    for slot in &mut out.0 {
        *slot = pixel;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test scaffolding")]
#[allow(
    clippy::field_reassign_with_default,
    reason = "Entity has nested fields; post-init assignment reads more clearly than struct-update syntax"
)]
mod tests {
    use super::*;

    #[test]
    fn breath_brightness_peaks_at_midcycle() {
        let trough = breath_brightness(Instant::from_millis(0));
        let peak = breath_brightness(Instant::from_millis(3_000));
        let trough_again = breath_brightness(Instant::from_millis(6_000));

        assert!(peak > trough, "peak {peak} must exceed trough {trough}");
        assert_eq!(peak, BRIGHTNESS_PEAK);
        // Trough is ~60% of peak = 61.
        assert_eq!(trough, 61);
        // One full cycle returns to trough.
        assert_eq!(trough, trough_again);
    }

    #[test]
    fn breath_brightness_stays_within_bounds() {
        for ms in 0..12_000u64 {
            let b = breath_brightness(Instant::from_millis(ms));
            assert!(
                b <= BRIGHTNESS_PEAK,
                "brightness {b} exceeds cap at ms {ms}"
            );
            assert!(b >= 61, "brightness {b} below trough at ms {ms}");
        }
    }

    #[test]
    fn render_leds_fills_every_pixel_with_same_colour() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut frame = LedFrame::default();
        render_leds(&entity, Instant::from_millis(3_000), &mut frame);
        let first = frame.0[0];
        assert_ne!(first, 0, "frame should not be black at peak brightness");
        for (i, px) in frame.0.iter().enumerate() {
            assert_eq!(*px, first, "pixel {i} diverged from pixel 0");
        }
    }

    #[test]
    fn emotion_changes_colour() {
        let mut entity = Entity::default();
        let mut frame = LedFrame::default();
        let now = Instant::from_millis(3_000); // peak brightness, deterministic

        entity.mind.affect.emotion = Emotion::Happy;
        render_leds(&entity, now, &mut frame);
        let happy = frame.0[0];

        entity.mind.affect.emotion = Emotion::Sad;
        render_leds(&entity, now, &mut frame);
        let sad = frame.0[0];

        entity.mind.affect.emotion = Emotion::Sleepy;
        render_leds(&entity, now, &mut frame);
        let sleepy = frame.0[0];

        assert_ne!(happy, sad, "happy and sad should produce different pixels");
        assert_ne!(
            happy, sleepy,
            "happy and sleepy should produce different pixels"
        );
        assert_ne!(
            sad, sleepy,
            "sad and sleepy should produce different pixels"
        );
    }

    #[test]
    fn breath_envelope_modulates_brightness_across_cycle() {
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy; // amber: high R, moderate G, low B
        let mut frame = LedFrame::default();

        render_leds(&entity, Instant::from_millis(0), &mut frame);
        let trough_r = (frame.0[0] >> 11) & 0x1F;

        render_leds(&entity, Instant::from_millis(3_000), &mut frame);
        let peak_r = (frame.0[0] >> 11) & 0x1F;

        assert!(
            peak_r > trough_r,
            "red channel should be brighter at breath peak ({peak_r}) than trough ({trough_r})"
        );
    }

    #[test]
    fn rgb565_packing_matches_reference_formula() {
        // Pure red 888 -> 0xF800 in 565.
        assert_eq!(rgb888_to_565(0xFF, 0x00, 0x00), 0xF800);
        // Pure green -> 0x07E0.
        assert_eq!(rgb888_to_565(0x00, 0xFF, 0x00), 0x07E0);
        // Pure blue -> 0x001F.
        assert_eq!(rgb888_to_565(0x00, 0x00, 0xFF), 0x001F);
        // Black stays black.
        assert_eq!(rgb888_to_565(0, 0, 0), 0);
    }

    #[test]
    fn scale_channel_caps_at_input_when_multiplier_255() {
        assert_eq!(scale_channel(255, 255), 255);
        assert_eq!(scale_channel(100, 255), 100);
    }

    #[test]
    fn scale_channel_zero_multiplier_is_zero() {
        assert_eq!(scale_channel(255, 0), 0);
    }

    #[test]
    fn as_u16_slice_matches_underlying_pixels() {
        let frame = LedFrame([0xF800; LED_COUNT]);
        let slice = frame.as_u16_slice();
        assert_eq!(slice.len(), LED_COUNT);
        for v in slice {
            assert_eq!(*v, 0xF800);
        }
    }

    #[test]
    fn default_frame_is_all_zeros() {
        let frame = LedFrame::default();
        for px in &frame.0 {
            assert_eq!(*px, 0);
        }
    }

    #[test]
    fn listen_intent_overrides_emotion_palette() {
        // `WakeOnVoice` pins emotion to Happy when sustained voice is
        // detected. With Intent::Listen also set (by LookAtSound), the
        // LED palette must shift to teal — not amber.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        entity.mind.intent = Intent::Listen;
        let mut frame = LedFrame::default();
        render_leds(&entity, Instant::from_millis(3_000), &mut frame);
        let listen_px = frame.0[0];

        entity.mind.intent = Intent::Idle;
        render_leds(&entity, Instant::from_millis(3_000), &mut frame);
        let happy_px = frame.0[0];

        assert_ne!(
            listen_px, happy_px,
            "Listen intent should produce a different colour than the underlying Happy"
        );
    }

    #[test]
    fn hearing_loud_pins_brightness_to_peak() {
        // At the breath trough, brightness should normally dip to ~60%.
        // With Intent::HearingLoud set, the dim is suppressed and the
        // pixel is at peak brightness regardless of breath phase.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Surprised;
        let mut frame = LedFrame::default();

        // Trough phase, no intent override.
        render_leds(&entity, Instant::from_millis(0), &mut frame);
        let trough_b = frame.0[0] & 0x1F;

        // Same trough phase, HearingLoud override.
        entity.mind.intent = Intent::HearingLoud;
        render_leds(&entity, Instant::from_millis(0), &mut frame);
        let loud_b = frame.0[0] & 0x1F;

        assert!(
            loud_b > trough_b,
            "HearingLoud should pin brightness above the breath trough ({trough_b} → {loud_b})"
        );
    }

    #[test]
    fn other_intents_do_not_override_emotion_palette() {
        // BeingPet / PickedUp / Shaken / Tilted / Idle / HearingLoud
        // all defer to the emotion palette for colour. Verify by
        // comparing each against Idle for the same emotion.
        let mut entity = Entity::default();
        entity.mind.affect.emotion = Emotion::Happy;
        let mut frame = LedFrame::default();
        let now = Instant::from_millis(3_000); // peak brightness, deterministic

        entity.mind.intent = Intent::Idle;
        render_leds(&entity, now, &mut frame);
        let baseline = frame.0[0];

        for intent in [
            Intent::BeingPet,
            Intent::PickedUp,
            Intent::Shaken,
            Intent::Tilted,
            // HearingLoud overrides brightness, not palette — same
            // hue at peak brightness.
            Intent::HearingLoud,
        ] {
            entity.mind.intent = intent;
            render_leds(&entity, now, &mut frame);
            assert_eq!(
                frame.0[0], baseline,
                "intent {intent:?} unexpectedly changed the palette"
            );
        }
    }
}
