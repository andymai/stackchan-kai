//! stackchan-firmware binary entry point.
//!
//! v0.1.0 scaffold. The actual hardware integration (esp-hal init, AXP2101
//! bring-up, SPI LCD driver, render loop) lands in v0.2.0 once the esp
//! toolchain is sourced in the local development environment.
//!
//! This stub exists so the workspace compiles on host CI (which does not
//! have the esp toolchain) while the firmware crate carries its own
//! real-hardware compilation path under `--target xtensa-esp32s3-none-elf`.

// The firmware crate will eventually be `#![no_std]` with a `#[no_main]`
// entry point; keeping it as a host-compilable stub until the esp toolchain
// lands keeps the CI story simple.

#[cfg(target_arch = "xtensa")]
compile_error!(
    "v0.1.0 is a scaffold; wire esp-hal init in a follow-up PR. Until then, \
     firmware cross-check runs `cargo check`, which skips binary lowering."
);

fn main() {
    println!(
        "stackchan-firmware v{} -- scaffold binary. See src/main.rs for the \
         real entry point (v0.2.0).",
        env!("CARGO_PKG_VERSION")
    );
}
