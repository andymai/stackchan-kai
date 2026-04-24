//! Snapshot test for `Avatar::draw`.
//!
//! Renders the default `Avatar` into an in-memory 320x240 RGB565 framebuffer
//! and asserts on a handful of hand-picked pixels: the eye centers, the
//! background between the eyes, the mouth line, and a screen corner. This
//! catches regressions in the draw code without needing hardware. It does
//! *not* do a full pixel-hash snapshot — the set of asserted pixels is small
//! enough to survive reasonable geometry tweaks.

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use stackchan_core::Avatar;
use stackchan_sim::Framebuffer;

/// LCD canvas width the firmware targets.
const WIDTH: u32 = 320;
/// LCD canvas height the firmware targets.
const HEIGHT: u32 = 240;

#[test]
fn default_avatar_renders_expected_pixels() {
    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    Avatar::default()
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    // Eye centers: default Avatar places left eye at (100, 110), right at
    // (220, 110), both filled black ellipses of radius 25.
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
    Avatar::default()
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
    let mut avatar = Avatar::default();
    avatar.mouth.mouth_open = 1.0;
    let mut open = Framebuffer::new(WIDTH, HEIGHT);
    avatar
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
    Avatar::default()
        .draw(&mut default_fb)
        .expect("Framebuffer DrawTarget is Infallible");

    let mut avatar = Avatar::default();
    avatar.mouth.mouth_open = 0.0;
    let mut zero_fb = Framebuffer::new(WIDTH, HEIGHT);
    avatar
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
