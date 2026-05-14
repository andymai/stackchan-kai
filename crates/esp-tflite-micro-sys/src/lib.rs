//! FFI bindings to esp-tflite-micro + esp-nn for ESP32-S3
//! keyword-spotting inference.
//!
//! **Foundation slice.** This crate ships the build plumbing
//! (vendored sources, cc-rs invocation, S3-specific gcc pin,
//! ESP-DSP / PIE extension assembly, TFLM bare-metal port
//! surface) and one C-ABI shim that links `tflite::InitializeTarget()`
//! so the build path is verifiable end-to-end. The real surface —
//! a `MicroInterpreter` wrapper around the MixConv operator set
//! microWakeWord needs — lands in a follow-up PR.
//!
//! ## Why this exists
//!
//! Wake-word inference is the one deliberate exception to the
//! firmware crate's "no C/C++ in the binary" non-goal. Three
//! pure-Rust paths were evaluated against the published
//! microWakeWord operator set (`microflow`, `candle-core`,
//! custom MixConv interpreter); all three either lack the
//! resource-variable streaming machinery the model requires
//! (`microflow`: 8 of 20 ops), have no embedded backend
//! (`candle`), or give up ESP-NN's ~7× SIMD speedup the
//! ESP32-S3 PIE / DSP extensions deliver (custom interpreter).
//! `esp-tflite-micro` is what ESPHome ships in production and
//! lands microWakeWord inference in well under our 200 ms
//! latency budget.
//!
//! Detailed research + the four open-question spike outcomes
//! live in `.notes/arc-4b-inference-research.md` (gitignored;
//! conventional v0.x scratch path).
//!
//! ## Build host
//!
//! Only builds on the `xtensa-esp32s3-none-elf` target via the
//! `esp` Rust toolchain. Host builds intentionally fail — the
//! crate has nothing useful to expose on x86_64 and pretending
//! otherwise would just push the failure to wake-word task spawn
//! time.

#![no_std]
// `-sys` crates by definition cross the safe/unsafe boundary —
// every `extern "C"` declaration and every call into it through
// the public surface is `unsafe`. The workspace-wide
// `unsafe_code = "deny"` is the right default for Rust crates;
// here we narrow it to "allowed but each block needs a
// SAFETY comment" via the per-file allow + the clippy lint that
// flags missing safety docs.
#![allow(unsafe_code)]
#![warn(clippy::missing_safety_doc, clippy::undocumented_unsafe_blocks)]
// Spec-heavy reference module: TFLM, ESP-NN, MicroInterpreter,
// microWakeWord, MixConv, ESP-DSP / PIE, etc. appear throughout
// the docs. Per-occurrence backticking would bury the prose.
// Same pattern as `stackchan-net::blufi`'s module-level allow.
#![allow(clippy::doc_markdown)]

/// One-time TFLM port-layer initialisation. The default
/// `tflite::InitializeTarget()` is `{}` on this port; the call
/// exists so the linker can't strip the port-surface symbols
/// out of the staticlib.
///
/// Safe to call multiple times.
///
/// # Safety
///
/// The underlying C++ implementation is `extern "C"` and does
/// nothing on this port; the `unsafe` marker is mechanical, not
/// load-bearing.
pub fn init() {
    // SAFETY: the underlying C++ `tflite::InitializeTarget()`
    // is an empty `{}` body on this port — no preconditions, no
    // global state mutation, no observable side effects.
    unsafe { esp_tflite_micro_init() };
}

unsafe extern "C" {
    /// `tflite::InitializeTarget()` wrapped behind a C-ABI symbol.
    fn esp_tflite_micro_init();
}
