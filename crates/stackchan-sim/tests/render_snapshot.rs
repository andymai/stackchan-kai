//! Snapshot test for `Entity::draw`.
//!
//! Renders the default `Entity` into an in-memory 320x240 RGB565 framebuffer
//! and asserts on a handful of hand-picked pixels: the eye centers, the
//! background between the eyes, the mouth line, and a screen corner. This
//! catches regressions in the draw code without needing hardware. It does
//! *not* do a full pixel-hash snapshot — the set of asserted pixels is small
//! enough to survive reasonable geometry tweaks.

#![allow(
    clippy::expect_used,
    reason = "test-only: framebuffer DrawTarget is Infallible and the Director \
              registry capacity is a compile-time constant in this fixture"
)]

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use stackchan_core::modifiers::StyleFromEmotion;
use stackchan_core::{
    BubbleState, Decorator, DecoratorState, Director, Emotion, Entity, EyePhase, Instant,
};
use stackchan_sim::Framebuffer;

/// LCD canvas width the firmware targets.
const WIDTH: u32 = 320;
/// LCD canvas height the firmware targets.
const HEIGHT: u32 = 240;

#[test]
fn default_avatar_renders_expected_pixels() {
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    // Eye centers: default Entity places left eye at (100, 110), right at
    // (220, 110), both filled black rounded squares of half-side 20.
    assert_eq!(fb.pixel(100, 110), Some(Rgb565::BLACK), "left eye center");
    assert_eq!(fb.pixel(220, 110), Some(Rgb565::BLACK), "right eye center");

    // Midpoint between the eyes: well outside either ellipse, should be
    // the white background.
    assert_eq!(
        fb.pixel(160, 110),
        Some(Rgb565::WHITE),
        "background between eyes"
    );

    // Corners: nothing drawn here, must be the clear color.
    assert_eq!(fb.pixel(0, 0), Some(Rgb565::WHITE), "top-left corner");
    assert_eq!(
        fb.pixel(WIDTH - 1, HEIGHT - 1),
        Some(Rgb565::WHITE),
        "bottom-right corner"
    );

    // Mouth at y=180, weight=0 → horizontal pink line. The draw code uses
    // a 3-pixel stroke, so x=160 on the line's path must be the mouth color.
    let mouth_pink = Rgb565::new(30, 32, 16);
    assert_eq!(fb.pixel(160, 180), Some(mouth_pink), "mouth center");
}

#[test]
fn out_of_bounds_reads_return_none() {
    let fb = Framebuffer::new(WIDTH, HEIGHT);
    assert!(fb.pixel(WIDTH, 0).is_none());
    assert!(fb.pixel(0, HEIGHT).is_none());
}

#[test]
fn audio_open_lifts_mouth_above_resting_line() {
    // Default avatar has `weight = 0` + `mouth_curve = 0` → 3 px
    // horizontal pink stroke centered on y=180. With `mouth_open =
    // 1.0` the audio-driven ellipse grows to ~40 px tall (radius_y
    // = 20), painting mouth colour at centre column y values well
    // outside the 3 px stroke.
    let mouth_pink = Rgb565::new(30, 32, 16);

    // Pre-condition: centre column at y=175 is background on the
    // resting mouth (3 px stroke covers y=179..=181 only).
    let mut resting = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut resting)
        .expect("Framebuffer DrawTarget is Infallible");
    assert_eq!(
        resting.pixel(160, 175),
        Some(Rgb565::WHITE),
        "pre-condition: y=175 is background when mouth_open = 0.0"
    );
    assert_eq!(
        resting.pixel(160, 185),
        Some(Rgb565::WHITE),
        "pre-condition: y=185 is background when mouth_open = 0.0"
    );

    // With full-scale audio (mouth_open = 1.0) the ellipse spans
    // roughly y=160..=199, comfortably including both y=175 and y=185.
    let mut avatar = Entity::default();
    avatar.face.mouth.mouth_open = 1.0;
    let mut open = Framebuffer::new(WIDTH, HEIGHT);
    avatar
        .face
        .draw(&mut open)
        .expect("Framebuffer DrawTarget is Infallible");

    assert_eq!(
        open.pixel(160, 175),
        Some(mouth_pink),
        "mouth_open=1.0 should paint y=175"
    );
    assert_eq!(
        open.pixel(160, 185),
        Some(mouth_pink),
        "mouth_open=1.0 should paint y=185"
    );

    // Mouth centre stays pink.
    assert_eq!(open.pixel(160, 180), Some(mouth_pink), "mouth centre");
}

/// Render every `Emotion` variant through `StyleFromEmotion` and assert
/// the resulting frame differs from the neutral baseline. Catches a
/// palette row that accidentally matches `Neutral`'s style — visually
/// invisible, but a regression we'd otherwise only spot on hardware.
///
/// `open_weight` is a *cap* that `Blink` writes into `eye.weight` on
/// every open transition; in a static one-frame snapshot we apply it
/// here ourselves to mirror Blink's at-rest behavior — otherwise
/// Sleepy / Boring (which differ from Neutral primarily through
/// dynamic-only fields) would read as identical.
#[test]
fn every_emotion_renders_distinguishable_frame() {
    fn render(emotion: Emotion) -> Framebuffer {
        let mut fb = Framebuffer::new(WIDTH, HEIGHT);
        let mut entity = Entity::default();
        entity.mind.affect.emotion = emotion;
        let mut style = StyleFromEmotion::new();
        let mut director = Director::new();
        director
            .add_modifier(&mut style)
            .expect("Director registry has room for one modifier");
        // Two ticks past the transition window so the style settles.
        director.run(&mut entity, Instant::from_millis(0));
        director.run(
            &mut entity,
            Instant::from_millis(StyleFromEmotion::TRANSITION_MS + 1),
        );
        // Apply Blink's open-state contract: at rest, eye.weight is
        // pinned to open_weight. Without this, dynamic-only style
        // differences (blink rate, breath depth, lid droop) would
        // collapse onto identical static frames.
        entity.face.left_eye.weight = entity.face.left_eye.open_weight;
        entity.face.right_eye.weight = entity.face.right_eye.open_weight;
        entity
            .face
            .draw(&mut fb)
            .expect("Framebuffer DrawTarget is Infallible");
        fb
    }

    let neutral_fb = render(Emotion::Neutral);

    for &emotion in Emotion::ALL {
        if emotion == Emotion::Neutral {
            continue;
        }
        let fb = render(emotion);
        assert_ne!(
            fb.as_slice(),
            neutral_fb.as_slice(),
            "{emotion:?} rendered identically to Neutral — palette row may have collided"
        );
    }
}

/// Each [`Decorator`] variant must render visibly distinct from the
/// no-decorator baseline. The base face is identical across runs;
/// only the overlay layer differs.
#[test]
fn every_decorator_renders_distinguishable_overlay() {
    let mut baseline = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut baseline)
        .expect("Framebuffer DrawTarget is Infallible");

    for &kind in Decorator::ALL {
        let mut entity = Entity::default();
        entity.face.decorator = Some(DecoratorState {
            kind,
            // Far in the future — we draw the overlay regardless of
            // expiry; expiry is the modifier's job, not the renderer's.
            expires_at: Instant::from_millis(u64::MAX / 2),
        });
        let mut fb = Framebuffer::new(WIDTH, HEIGHT);
        entity
            .face
            .draw(&mut fb)
            .expect("Framebuffer DrawTarget is Infallible");
        assert_ne!(
            fb.as_slice(),
            baseline.as_slice(),
            "{kind:?} should paint pixels outside the no-decorator baseline"
        );
    }
}

/// Two distinct decorators must produce visibly distinct frames. Catches
/// a draw-routine collision (e.g. Heart and Sweat anchored at the same
/// pixel range with the same colour palette).
#[test]
fn distinct_decorators_render_distinct_frames() {
    fn render(kind: Decorator) -> Framebuffer {
        let mut entity = Entity::default();
        entity.face.decorator = Some(DecoratorState {
            kind,
            expires_at: Instant::from_millis(u64::MAX / 2),
        });
        let mut fb = Framebuffer::new(WIDTH, HEIGHT);
        entity
            .face
            .draw(&mut fb)
            .expect("Framebuffer DrawTarget is Infallible");
        fb
    }

    for (i, &a) in Decorator::ALL.iter().enumerate() {
        let fb_a = render(a);
        for &b in &Decorator::ALL[i + 1..] {
            let fb_b = render(b);
            assert_ne!(
                fb_a.as_slice(),
                fb_b.as_slice(),
                "{a:?} and {b:?} render identically — overlay anchors / colours have collided"
            );
        }
    }
}

/// A populated speech bubble must render visibly different from the
/// no-bubble baseline, regardless of which face is showing underneath.
#[test]
fn bubble_overlay_paints_pixels_outside_baseline() {
    let mut baseline = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut baseline)
        .expect("Framebuffer DrawTarget is Infallible");

    let mut entity = Entity::default();
    entity.face.bubble = Some(BubbleState {
        text: "hi",
        // Far in the future — the renderer draws regardless of expiry.
        expires_at: Instant::from_millis(u64::MAX / 2),
    });
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    entity
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");
    assert_ne!(
        fb.as_slice(),
        baseline.as_slice(),
        "bubble overlay should paint pixels outside the no-bubble baseline"
    );
}

/// A bubble with text long enough to overflow the maximum frame width
/// must still render — the draw code truncates rather than panicking
/// or overflowing the framebuffer.
#[test]
fn bubble_overlay_handles_oversized_text() {
    let mut entity = Entity::default();
    entity.face.bubble = Some(BubbleState {
        // Far longer than the bubble can hold (~28 chars at FONT_10X20
        // on a 320 px frame minus padding); the draw code truncates.
        text: "this is a deliberately very long speech bubble text that overflows",
        expires_at: Instant::from_millis(u64::MAX / 2),
    });
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    entity
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");
    // No assertion on pixels beyond "draw didn't panic and produced
    // something different from baseline" — visual truncation is
    // covered by the no-overflow guarantee inside `draw_bubble`.
    let mut baseline = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut baseline)
        .expect("Framebuffer DrawTarget is Infallible");
    assert_ne!(fb.as_slice(), baseline.as_slice());
}

#[test]
fn cheek_renders_two_tone_gradient_when_blush_set() {
    // High blush so both rings paint clearly distinguishable colors:
    // inner core at full intensity (≈ MOUTH_COLOR), outer halo at half
    // intensity (a much lighter pink). Both must be non-white, and the
    // two colors must differ.
    let mut avatar = Entity::default();
    avatar.face.style.cheek_blush = 200;

    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    avatar
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    // Cheek geometry under the LEFT eye at (100, 110) radius_y=20 with
    // CHEEK_VERTICAL_GAP=6 → cheek_top = 136. Outer 22 px halo and inner
    // 12 px core both centre on (100, 147).
    let centre = fb.pixel(100, 147);
    // (92, 147) is 8 px from the centre — well inside the 11 px halo and
    // well outside the 6 px core.
    let halo = fb.pixel(92, 147);

    // (85, 147) is 15 px from the centre — outside the 11 px halo entirely,
    // so it must be the white background.
    assert_eq!(
        fb.pixel(85, 147),
        Some(Rgb565::WHITE),
        "halo must not bleed past its 11 px radius"
    );
    assert_ne!(centre, Some(Rgb565::WHITE), "cheek core should be pink");
    assert_ne!(halo, Some(Rgb565::WHITE), "cheek halo should be visible");
    assert_ne!(
        centre, halo,
        "core and halo must paint distinguishable colors so the gradient reads as two-tone"
    );
}

#[test]
fn closed_eye_renders_as_upward_smile_arc() {
    // Closed phase = an upward parabolic arc spanning the open eye's
    // full width (40 px), apex lifted ~10 px above baseline. For the
    // LEFT eye at (100, 110) radius 20 the curve runs from (80, 110)
    // up through (100, 100) back to (120, 110), drawn with the same
    // 5 px polyline stroke as the Happy/Sad eye_curve arcs. Polyline
    // thick-stroke joins notch at the exact apex vertex, so the densest
    // pixel sits one row below the geometric apex.
    let mut avatar = Entity::default();
    avatar.face.left_eye.phase = EyePhase::Closed;
    avatar.face.right_eye.phase = EyePhase::Closed;

    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    avatar
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    assert_eq!(
        fb.pixel(100, 101),
        Some(Rgb565::BLACK),
        "smile-arc apex stroke must be drawn one row below geometric apex"
    );
    assert_eq!(
        fb.pixel(82, 110),
        Some(Rgb565::BLACK),
        "near-endpoint must sit on the baseline stroke"
    );
    assert_eq!(
        fb.pixel(100, 110),
        Some(Rgb565::WHITE),
        "below the lifted arc must be background, not the old horizontal line"
    );
}

#[test]
fn open_eye_renders_as_rounded_square() {
    // The open neutral eye is a 40×40 filled rounded rectangle with a
    // 10 px corner radius. The bounding-box corner pixel (80, 90) sits
    // outside the rounded corner's quarter-circle (distance from corner
    // arc centre (90, 100) ≈ 14 px > 10 px radius), so it must be
    // background. The corner-arc centre itself is firmly inside the
    // shape and must be eye-black. This is what distinguishes a rounded
    // square from a plain rectangle (where the corner pixel would be
    // black) or an ellipse (where the bounding-box corner is also
    // background but the centre arc shape differs).
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    assert_eq!(
        fb.pixel(80, 90),
        Some(Rgb565::WHITE),
        "bounding-box corner must be carved out by the 10 px rounded corner"
    );
    assert_eq!(
        fb.pixel(90, 100),
        Some(Rgb565::BLACK),
        "rounded-corner arc centre must lie inside the shape"
    );
    // Side midpoints of the bounding box are flat edges of the rounded
    // square — must be solidly inside.
    assert_eq!(
        fb.pixel(100, 90),
        Some(Rgb565::BLACK),
        "top edge midpoint must be inside the shape"
    );
    assert_eq!(
        fb.pixel(80, 110),
        Some(Rgb565::BLACK),
        "left edge midpoint must be inside the shape"
    );
}

#[test]
fn audio_open_zero_renders_identical_to_default_avatar() {
    // Backwards-compat: a freshly-defaulted avatar (mouth_open = 0.0)
    // must render exactly as it did before this feature landed.
    let mut default_fb = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut default_fb)
        .expect("Framebuffer DrawTarget is Infallible");

    let mut avatar = Entity::default();
    avatar.face.mouth.mouth_open = 0.0;
    let mut zero_fb = Framebuffer::new(WIDTH, HEIGHT);
    avatar
        .face
        .draw(&mut zero_fb)
        .expect("Framebuffer DrawTarget is Infallible");

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            assert_eq!(
                default_fb.pixel(x, y),
                zero_fb.pixel(x, y),
                "pixel ({x}, {y}) should match default avatar"
            );
        }
    }
}
