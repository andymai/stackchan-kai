//! Host-side avatar visualiser.
//!
//! Runs the canonical firmware modifier stack
//! (`Blink` → `Breath` → `IdleDrift` → `IdleSway`) against a wall-clock
//! source and renders the resulting `Entity` into a 320×240 window via
//! `egui` + `winit` at ~30 FPS. Lets behavior changes iterate in
//! sub-second cycles instead of the 30 s+ build → flash → boot loop.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p stackchan-sim --bin viz --features viz
//! ```
//!
//! Side panel exposes:
//! - FPS counter (effective)
//! - Emotion-override buttons (cycle through every variant)
//! - `IdleDrift` seed input — re-creates `IdleDrift::with_seed(...)`,
//!   useful for "did the seed actually change anything?" sanity checks
//!
//! Not a regression test surface — the headless integration tests in
//! `lib.rs` and `tests/` are still the canonical asserts. This binary's
//! job is interactive iteration.

// Dev-tool binary: relax the workspace's library-grade lints so app
// state structs and one-off cast helpers don't drown the file in
// noise. Library code (everything outside `src/bin/`) stays strict.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    missing_docs,
    clippy::missing_docs_in_private_items
)]

use std::cell::Cell;
use std::time::Instant as StdInstant;

use eframe::egui;
use embedded_graphics::pixelcolor::RgbColor;
use stackchan_core::modifiers::{Blink, Breath, IdleDrift, IdleSway};
use stackchan_core::{Clock, Emotion, Entity, Instant, Modifier};
use stackchan_sim::Framebuffer;

/// Display dimensions match the firmware's ILI9342C panel.
const FB_WIDTH: u32 = 320;
const FB_HEIGHT: u32 = 240;

/// Wall-clock source for the modifier stack. `std::time::Instant` is
/// monotonic; we anchor to construction time so `now()` returns elapsed
/// milliseconds since startup — exactly the shape `stackchan_core`
/// expects from its [`Clock`] trait.
struct WallClock {
    start: StdInstant,
    /// `Cell` because `Clock::now` takes `&self`, matching `FakeClock`'s shape.
    _phantom: Cell<()>,
}

impl WallClock {
    fn new() -> Self {
        Self {
            start: StdInstant::now(),
            _phantom: Cell::new(()),
        }
    }
}

impl Clock for WallClock {
    fn now(&self) -> Instant {
        let elapsed_ms = self.start.elapsed().as_millis();
        // Saturating cast: a 32-bit firmware wraps after ~49 days; we
        // never run a viz session that long.
        let ms = u64::try_from(elapsed_ms).unwrap_or(u64::MAX);
        Instant::from_millis(ms)
    }
}

/// Convert the firmware-shape `Framebuffer<Rgb565>` into an `egui::ColorImage`.
/// `Rgb565` packs 5R/6G/5B into 16 bits; we expand to 8-bit per channel by
/// left-shift + low-bit replication so 100% intensity round-trips correctly
/// (`r=31` → `0xFF`, not `0xF8`).
fn framebuffer_to_color_image(fb: &Framebuffer) -> egui::ColorImage {
    let size = [FB_WIDTH as usize, FB_HEIGHT as usize];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for px in fb.as_slice() {
        let r5 = px.r();
        let g6 = px.g();
        let b5 = px.b();
        let r = (r5 << 3) | (r5 >> 2);
        let g = (g6 << 2) | (g6 >> 4);
        let b = (b5 << 3) | (b5 >> 2);
        pixels.push(egui::Color32::from_rgb(r, g, b));
    }
    egui::ColorImage::new(size, pixels)
}

struct VizApp {
    avatar: Entity,
    fb: Framebuffer,
    clock: WallClock,
    blink: Blink,
    breath: Breath,
    drift: IdleDrift,
    sway: IdleSway,
    drift_seed: u32,
    emotion_override: Option<Emotion>,
    texture: Option<egui::TextureHandle>,
    last_render: StdInstant,
    frame_count: u64,
    last_fps_window: StdInstant,
    effective_fps: f32,
}

impl VizApp {
    fn new() -> Self {
        let drift_seed = 0xDEAD_BEEF;
        Self {
            avatar: Entity::default(),
            fb: Framebuffer::new(FB_WIDTH, FB_HEIGHT),
            clock: WallClock::new(),
            blink: Blink::new(),
            breath: Breath::new(),
            drift: IdleDrift::with_seed(core::num::NonZeroU32::new(drift_seed).unwrap()),
            sway: IdleSway::new(),
            drift_seed,
            emotion_override: None,
            texture: None,
            last_render: StdInstant::now(),
            frame_count: 0,
            last_fps_window: StdInstant::now(),
            effective_fps: 0.0,
        }
    }

    fn tick_modifiers(&mut self) {
        if let Some(e) = self.emotion_override {
            self.avatar.mind.affect.emotion = e;
        }
        self.avatar.tick.now = self.clock.now();
        self.blink.update(&mut self.avatar);
        self.breath.update(&mut self.avatar);
        self.drift.update(&mut self.avatar);
        self.sway.update(&mut self.avatar);
    }

    fn redraw_framebuffer(&mut self) {
        // Clear + redraw. Face::draw returns Infallible on Framebuffer.
        let _ = self.avatar.face.draw(&mut self.fb);
    }

    fn update_fps_counter(&mut self) {
        self.frame_count += 1;
        let elapsed = self.last_fps_window.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.effective_fps = self.frame_count as f32 / elapsed;
            self.frame_count = 0;
            self.last_fps_window = StdInstant::now();
        }
    }
}

impl eframe::App for VizApp {
    /// Mutation phase. eframe 0.34 split `update` into `logic` (mutation,
    /// no painting) and `ui` (painting, no mutation). All state updates
    /// happen here; the `ui` impl below only paints.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Cap effective tick to ~33 ms (~30 FPS). egui defaults to a
        // higher refresh than we need — uploading a fresh ColorImage at
        // monitor refresh rate (60-120 Hz) is wasteful.
        if self.last_render.elapsed().as_millis() >= 33 {
            self.tick_modifiers();
            self.redraw_framebuffer();
            let img = framebuffer_to_color_image(&self.fb);
            match &mut self.texture {
                Some(tex) => tex.set(img, egui::TextureOptions::NEAREST),
                None => {
                    self.texture =
                        Some(ctx.load_texture("avatar-fb", img, egui::TextureOptions::NEAREST));
                }
            }
            self.update_fps_counter();
            self.last_render = StdInstant::now();
        }

        // Force the next frame so we keep ticking even when the user
        // isn't moving the mouse — egui is lazy by default.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    /// Paint phase. Paint UI only; do not mutate visualizer state here.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Side panel: live controls.
        egui::Panel::right("controls").show_inside(ui, |ui| {
            ui.heading("controls");
            ui.label(format!("effective fps: {:.1}", self.effective_fps));
            ui.label(format!("now: {} ms", self.clock.now().as_millis()));
            ui.separator();

            ui.label("emotion override:");
            ui.horizontal_wrapped(|ui| {
                let emotions = [
                    ("none", None),
                    ("Neutral", Some(Emotion::Neutral)),
                    ("Happy", Some(Emotion::Happy)),
                    ("Sad", Some(Emotion::Sad)),
                    ("Sleepy", Some(Emotion::Sleepy)),
                    ("Surprised", Some(Emotion::Surprised)),
                ];
                for (label, emotion) in emotions {
                    if ui
                        .selectable_label(self.emotion_override == emotion, label)
                        .clicked()
                    {
                        self.emotion_override = emotion;
                    }
                }
            });
            ui.separator();

            ui.label("idle-drift seed:");
            ui.horizontal(|ui| {
                ui.label(format!("0x{:08X}", self.drift_seed));
                if ui.button("re-seed").clicked() {
                    // Pseudo-randomise via wall clock — gives the user
                    // a quick way to reproduce the firmware's
                    // boot-sampled-RNG behavior interactively.
                    self.drift_seed = self.clock.now().as_millis() as u32 | 1;
                    self.drift =
                        IdleDrift::with_seed(core::num::NonZeroU32::new(self.drift_seed).unwrap());
                }
            });
            ui.separator();

            ui.label("avatar fields:");
            ui.label(format!("emotion: {:?}", self.avatar.mind.affect.emotion));
            ui.label(format!(
                "left eye: ({}, {}) w={}",
                self.avatar.face.left_eye.center.x,
                self.avatar.face.left_eye.center.y,
                self.avatar.face.left_eye.weight,
            ));
            ui.label(format!(
                "mouth: w={} open={:.2}",
                self.avatar.face.mouth.weight, self.avatar.face.mouth.mouth_open,
            ));
        });

        // Central panel: the framebuffer image, scaled 2× for legibility.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(tex) = &self.texture {
                let display_size = egui::Vec2::new((FB_WIDTH * 2) as f32, (FB_HEIGHT * 2) as f32);
                ui.image((tex.id(), display_size));
            } else {
                ui.label("warming up...");
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([(FB_WIDTH * 2) as f32 + 240.0, (FB_HEIGHT * 2) as f32 + 40.0])
            .with_title("stackchan-viz"),
        ..Default::default()
    };
    eframe::run_native(
        "stackchan-viz",
        options,
        Box::new(|_cc| Ok(Box::new(VizApp::new()))),
    )
}
