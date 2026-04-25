//! Shared library surface of the StackChan firmware — modules that the
//! main binary (`src/main.rs`) and any `examples/*.rs` binary both use.
//!
//! Each binary still carries its own `#[panic_handler]`, app-descriptor
//! anchor, heap init, and `#[esp_rtos::main]` entry point; this crate
//! only hosts the reusable device-driver + task-infrastructure modules.
//!
//! Unsafe code is isolated inside specific binary files (the app-desc
//! anchor — see `main.rs` / `examples/bench.rs`). The library surface
//! itself is `#![deny(unsafe_code)]`.

#![no_std]
#![deny(unsafe_code)]
// esp-rtos runs a single-core executor on this chip; `Send`-bounded
// futures aren't meaningful here. The nursery lint fires on every task.
#![allow(clippy::future_not_send)]

extern crate alloc;

pub mod ambient;
pub mod audio;
pub mod board;
pub mod button;
pub mod camera;
pub mod clock;
pub mod framebuffer;
pub mod head;
pub mod imu;
pub mod ir;
pub mod leds;
pub mod power;
pub mod touch;
pub mod wallclock;
