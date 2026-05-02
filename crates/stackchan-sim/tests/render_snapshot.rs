//! Snapshot test for `Entity::draw`.
//!
//! Renders the default `Entity` into an in-memory 320x240 RGB565 framebuffer
//! and asserts on a handful of hand-picked pixels: the eye centers, the
//! background between the eyes, the mouth line, and a screen corner. This
//! catches regressions in the draw code without needing hardware. It does
//! *not* do a full pixel-hash snapshot — the set of asserted pixels is small
//! enough to survive reasonable geometry tweaks.

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use stackchan_core::Entity;
use stackchan_sim::Framebuffer;

/// LCD canvas width the firmware targets.
const WIDTH: u32 = 320;
/// LCD canvas height the firmware targets.
const HEIGHT: u32 = 240;

/// Warm off-white background color in `draw.rs` (`BG_COLOR`). Kept here
/// so the test reads as a single-source-of-truth check against the
/// rendered framebuffer.
const BG_COLOR: Rgb565 = Rgb565::new(30, 61, 29);

#[test]
fn default_avatar_renders_expected_pixels() {
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    Entity::default()
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    // Eye centers: default Entity places left eye at (100, 110), right at
    // (220, 110). Under the layered-eye look the inner core is pure
    // black, so the eye-center pixel is still BLACK.
    assert_eq!(fb.pixel(100, 110), Some(Rgb565::BLACK), "left eye center");
    assert_eq!(fb.pixel(220, 110), Some(Rgb565::BLACK), "right eye center");

    // Midpoint between the eyes: well outside either ellipse, should
    // show the warm off-white background.
    assert_eq!(
        fb.pixel(160, 110),
        Some(BG_COLOR),
        "background between eyes is BG_COLOR"
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
        Some(BG_COLOR),
        "pre-condition: y=175 is background when mouth_open = 0.0"
    );
    assert_eq!(
        resting.pixel(160, 185),
        Some(BG_COLOR),
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
