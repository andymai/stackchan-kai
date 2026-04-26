//! Golden test for `render_leds`: drives a full modifier stack against a
//! [`FakeClock`] and asserts that the resulting [`LedFrame`] matches the
//! emotion palette at peak brightness, dims under the breath envelope at
//! the trough, and tracks [`EmotionCycle`]'s rotation between variants.
//!
//! This is the sim counterpart to the host-side unit tests in
//! `stackchan_core::leds` — same function, exercised through the whole
//! modifier stack so regressions in any layer (emotion dispatch, breath
//! timing, palette lookup) show up here.

#![allow(
    clippy::field_reassign_with_default,
    reason = "Entity has many fields; post-init assignment is clearer than ..Default::default() here"
)]
#![allow(
    clippy::unwrap_used,
    reason = "test-only: Director registry capacity is a compile-time constant in this fixture, the unwraps can't fire"
)]

use stackchan_core::modifiers::{Blink, Breath, EmotionCycle, IdleDrift, StyleFromEmotion};
use stackchan_core::{BRIGHTNESS_PEAK, Director, Emotion, Entity, LED_COUNT, LedFrame};
use stackchan_core::{Instant, render_leds};

/// Drive the default v0.1.0 modifier stack via the `Director` up to
/// `modifiers_at` ms (33 ms steps so stateful modifiers see
/// intermediate times), then render LEDs and return the frame.
fn render_at(avatar: &mut Entity, modifiers_at: u64) -> LedFrame {
    let mut emotion_cycle = EmotionCycle::new();
    let mut style_from_emotion = StyleFromEmotion::new();
    let mut blink = Blink::new();
    let mut breath = Breath::new();
    let mut drift = IdleDrift::new();
    let mut director = Director::new();
    director.add_modifier(&mut emotion_cycle).unwrap();
    director.add_modifier(&mut style_from_emotion).unwrap();
    director.add_modifier(&mut blink).unwrap();
    director.add_modifier(&mut breath).unwrap();
    director.add_modifier(&mut drift).unwrap();

    let mut t = 0u64;
    while t <= modifiers_at {
        director.run(avatar, Instant::from_millis(t));
        t += 33;
    }

    let mut frame = LedFrame::default();
    render_leds(avatar, Instant::from_millis(modifiers_at), &mut frame);
    frame
}

#[test]
fn neutral_emotion_lights_soft_white_at_peak_breath() {
    let mut avatar = Entity::default();
    avatar.mind.affect.emotion = Emotion::Neutral;
    // No modifier pipeline — we want a pure palette probe.
    let mut frame = LedFrame::default();
    render_leds(&avatar, Instant::from_millis(3_000), &mut frame);

    // Soft white palette = 0xFFFFE8. At peak brightness (102/255) this
    // lands around (102, 102, ~93). Check each channel stays warm-white
    // (R >= G ~ B, with R and G saturated at the 5-bit / 6-bit limit).
    let px = frame.0[0];
    let r5 = (px >> 11) & 0x1F;
    let g6 = (px >> 5) & 0x3F;
    let b5 = px & 0x1F;
    assert!(r5 >= 10, "neutral R should be bright: got {r5}");
    assert!(g6 >= 20, "neutral G should be bright: got {g6}");
    assert!(b5 >= 9, "neutral B should be roughly matched: got {b5}");
    // All 12 pixels carry the same colour.
    for (i, p) in frame.0.iter().enumerate() {
        assert_eq!(*p, px, "pixel {i} diverged from pixel 0");
    }
}

#[test]
fn each_emotion_produces_a_distinct_pixel_at_peak() {
    // At the same peak-brightness moment, every emotion must light up
    // the ring with a visibly different RGB565 value. Protects against
    // palette collisions.
    let now = Instant::from_millis(3_000);
    let mut frame = LedFrame::default();
    let mut seen = Vec::new();
    for emotion in [
        Emotion::Neutral,
        Emotion::Happy,
        Emotion::Sad,
        Emotion::Sleepy,
        Emotion::Surprised,
    ] {
        let mut avatar = Entity::default();
        avatar.mind.affect.emotion = emotion;
        render_leds(&avatar, now, &mut frame);
        let px = frame.0[0];
        assert!(
            !seen.contains(&px),
            "{emotion:?} collided with a previous emotion palette entry ({px:#06x})"
        );
        seen.push(px);
    }
    assert_eq!(seen.len(), 5);
}

#[test]
fn breath_envelope_dims_the_ring_at_cycle_trough() {
    let mut avatar = Entity::default();
    avatar.mind.affect.emotion = Emotion::Happy; // amber has big R + G components
    let mut trough = LedFrame::default();
    let mut peak = LedFrame::default();
    render_leds(&avatar, Instant::from_millis(0), &mut trough);
    render_leds(&avatar, Instant::from_millis(3_000), &mut peak);

    // Red channel: peak should be strictly brighter than trough.
    let r_trough = (trough.0[0] >> 11) & 0x1F;
    let r_peak = (peak.0[0] >> 11) & 0x1F;
    assert!(
        r_peak > r_trough,
        "breath peak R ({r_peak}) should exceed trough R ({r_trough})"
    );
    // Trough envelope is ~60% of peak; in RGB565 5-bit R that's a
    // noticeable step, but the exact ratio varies with quantisation.
    // Just verify the direction here.
}

#[test]
fn render_leds_integrated_with_emotion_cycle() {
    // Drive the stack long enough that EmotionCycle has definitely
    // moved off Neutral, then confirm the LED palette follows.
    let mut avatar = Entity::default();
    let neutral_frame = {
        let mut f = LedFrame::default();
        render_leds(&avatar, Instant::from_millis(3_000), &mut f);
        f
    };

    // 30 seconds of sim time — EmotionCycle rotates every few seconds,
    // so the emotion will definitely have changed from Neutral.
    let later = render_at(&mut avatar, 30_000);
    assert_ne!(
        avatar.mind.affect.emotion,
        Emotion::Neutral,
        "EmotionCycle should have advanced off Neutral by 30s"
    );
    assert_ne!(
        later.0[0], neutral_frame.0[0],
        "LED palette should track EmotionCycle's emotion change"
    );

    // LED count invariant.
    assert_eq!(later.0.len(), LED_COUNT);
}

#[test]
fn frame_never_exceeds_brightness_cap() {
    // Sweep a full breath cycle and verify no channel exceeds what
    // BRIGHTNESS_PEAK allows. Tightens the host-side bound into a
    // sim-level guarantee.
    let mut avatar = Entity::default();
    avatar.mind.affect.emotion = Emotion::Surprised; // max R+G+B case
    let max_5bit = u16::from(BRIGHTNESS_PEAK) >> 3; // 5-bit R/B field
    let max_6bit = u16::from(BRIGHTNESS_PEAK) >> 2; // 6-bit G field
    let mut frame = LedFrame::default();
    for ms in (0..12_000).step_by(50) {
        render_leds(&avatar, Instant::from_millis(ms), &mut frame);
        for (i, px) in frame.0.iter().enumerate() {
            let r5 = (px >> 11) & 0x1F;
            let g6 = (px >> 5) & 0x3F;
            let b5 = px & 0x1F;
            assert!(
                r5 <= max_5bit,
                "ms {ms} px {i}: R channel {r5} exceeds 5-bit cap {max_5bit}"
            );
            assert!(
                g6 <= max_6bit,
                "ms {ms} px {i}: G channel {g6} exceeds 6-bit cap {max_6bit}"
            );
            assert!(
                b5 <= max_5bit,
                "ms {ms} px {i}: B channel {b5} exceeds 5-bit cap {max_5bit}"
            );
        }
    }
}
