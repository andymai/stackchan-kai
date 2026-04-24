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
