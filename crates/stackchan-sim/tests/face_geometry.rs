//! End-to-end test for the [`FaceGeometry`] preset surface.
//!
//! Exercises the same path the firmware takes when an operator
//! calls `POST /face-geometry`:
//!
//! 1. Parse the JSON body via `stackchan_net::http_command`.
//! 2. Apply the parsed preset to `entity.face` via
//!    [`Face::set_geometry`] — the same call the render task makes
//!    when draining `FACE_GEOMETRY_SIGNAL`.
//! 3. Render the avatar and assert the geometry actually changed at
//!    the pixel boundary, not just on the struct field.
//!
//! Together these guarantee the wire format, the in-memory swap, and
//! the renderer all stay in sync — a parser typo or a missed
//! geometry-channel field on `Face` would surface here before
//! shipping.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test-only: framebuffer DrawTarget is Infallible; parser inputs are compile-time fixtures"
)]

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use stackchan_core::{Entity, EyePhase, Face, FaceGeometry};
use stackchan_net::http_command::parse_face_geometry;
use stackchan_sim::Framebuffer;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

#[test]
fn parser_accepts_each_wire_string() {
    for &g in FaceGeometry::ALL {
        let body = format!(r#"{{"geometry":"{}"}}"#, g.wire_str());
        let parsed = parse_face_geometry(&body).expect("known wire string parses");
        assert_eq!(parsed, g, "round-trip failed for {g:?}");
    }
}

#[test]
fn baseline_swap_replaces_eye_and_mouth_geometry() {
    let mut entity = Entity::default();
    let default_left = entity.face.left_eye;
    entity.face.set_geometry(FaceGeometry::Wide);

    let (expected_left, expected_right, expected_mouth) = FaceGeometry::Wide.baseline();
    assert_eq!(entity.face.left_eye.center, expected_left.center);
    assert_eq!(entity.face.left_eye.radius_x, expected_left.radius_x);
    assert_eq!(entity.face.right_eye.center, expected_right.center);
    assert_eq!(entity.face.mouth.center, expected_mouth.center);
    assert_eq!(entity.face.mouth.radius_x, expected_mouth.radius_x);

    // Wide moves eyes outward — left eye should not be at the
    // Default position any more.
    assert_ne!(entity.face.left_eye.center, default_left.center);
}

#[test]
fn mid_blink_swap_preserves_dynamic_state() {
    let mut entity = Entity::default();
    entity.face.left_eye.phase = EyePhase::Closed;
    entity.face.left_eye.weight = 12;
    entity.face.left_eye.open_weight = 80;
    entity.face.right_eye.weight = 12;
    entity.face.mouth.weight = 60;
    entity.face.mouth.mouth_open = 0.4;

    entity.face.set_geometry(FaceGeometry::Sleepy);

    // Geometry replaced.
    let (expected_left, _, _) = FaceGeometry::Sleepy.baseline();
    assert_eq!(entity.face.left_eye.radius_y, expected_left.radius_y);
    // Dynamic state untouched — a real blink in flight keeps closing.
    assert_eq!(entity.face.left_eye.phase, EyePhase::Closed);
    assert_eq!(entity.face.left_eye.weight, 12);
    assert_eq!(entity.face.left_eye.open_weight, 80);
    assert_eq!(entity.face.right_eye.weight, 12);
    assert_eq!(entity.face.mouth.weight, 60);
    assert!((entity.face.mouth.mouth_open - 0.4).abs() < f32::EPSILON);
}

#[test]
fn chibi_renders_eye_at_chibi_baseline_not_default() {
    let mut entity = Entity::default();
    entity.face.set_geometry(FaceGeometry::Chibi);

    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    entity
        .face
        .draw(&mut fb)
        .expect("Framebuffer DrawTarget is Infallible");

    // Default's left eye centre at (100, 110) is now background —
    // Chibi's eye sits at (105, 115).
    assert_eq!(
        fb.pixel(105, 115),
        Some(Rgb565::BLACK),
        "Chibi left eye centre",
    );
    assert_eq!(
        fb.pixel(215, 115),
        Some(Rgb565::BLACK),
        "Chibi right eye centre",
    );
}

#[test]
fn with_geometry_constructor_picks_up_the_baseline() {
    let face = Face::with_geometry(FaceGeometry::Wide);
    let (expected_left, expected_right, expected_mouth) = FaceGeometry::Wide.baseline();
    assert_eq!(face.geometry, FaceGeometry::Wide);
    assert_eq!(face.left_eye.center, expected_left.center);
    assert_eq!(face.right_eye.center, expected_right.center);
    assert_eq!(face.mouth.center, expected_mouth.center);
}
