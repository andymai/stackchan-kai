// Per-file allow: build.rs documents protocol-spec terms (TFLM,
// ESP-NN, ESP-DSP / PIE, FreeRTOS, ESP-IDF, MixConv, MicroInterpreter)
// in nearly every sentence; backticking each occurrence would
// drown the prose. Same pattern as `stackchan-net::blufi`'s
// module-level allow.
#![allow(clippy::doc_markdown)]

//! Build script for `esp-tflite-micro-sys`.
//!
//! Compiles a focused vendored subset of `esp-nn` (Espressif's
//! int8 NN kernel library) + the bare-metal port surface of
//! `esp-tflite-micro` (TFLite Micro). This skeleton crate proves
//! the build path end-to-end:
//!
//! - the `esp` Rust toolchain wires `cc-rs` to the xtensa C++
//!   cross-compiler (resolved in `.notes/spike-tflm-cc/`);
//! - `esp-nn` extracts cleanly from ESP-IDF and the ESP32-S3
//!   PIE / DSP extension assembly is reachable (resolved in
//!   `.notes/spike-esp-nn/`);
//! - the TFLM port surface compiles bare-metal with no FreeRTOS
//!   or ESP-IDF dependencies (resolved by reading the
//!   `tensorflow/lite/micro/system_setup.cc` + `micro_log.cc` +
//!   `debug_log.cc` + `micro_time.cc` sources directly).
//!
//! A follow-up PR adds the actual MixConv operator kernels and a
//! `MicroInterpreter` wrapper; for this slice we just need the
//! build path to succeed and one C-ABI shim to link against the
//! TFLM symbols so a regression can't sneak into the build
//! plumbing.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vendor = manifest.join("vendor");
    let esp_nn = vendor.join("esp-nn");
    let tflm = vendor.join("esp-tflite-micro");

    // Shim + TFLM port archive comes first on the link line,
    // esp-nn second. cc-rs emits `cargo:rustc-link-lib=static=…`
    // in call order; GNU ld is single-pass and only pulls an
    // archive member if a reference has already been seen, so
    // the consumer archive must precede the provider. The shim
    // is the consumer of esp-nn's kernels; esp-nn provides.
    //
    // Two separate `cc::Build`s because the include paths are
    // disjoint — sharing one Build would force every TU to see
    // both header trees for no compilation benefit.
    compile_tflm_port_and_shim(&tflm, &manifest);
    compile_esp_nn(&esp_nn);
}

/// Pin `cc-rs` to the ESP32-S3-specific gcc binary the `esp`
/// toolchain ships. The default xtensa target dispatch picks
/// `xtensa-esp-elf-gcc` (generic), which fails on the S3's
/// ESP-DSP / PIE extension instructions (`ee.vldbc.16`,
/// `ee.zero.q`, `ee.vld.l.64.xp`, …) with "unknown opcode or
/// format name" at assembly time.
fn s3_build() -> cc::Build {
    let mut b = cc::Build::new();
    b.compiler("xtensa-esp32s3-elf-gcc");
    b
}

/// Compile the esp-nn kernels we care about for the streaming
/// MixConv operator set. The `*_ansi.c` files are the reference
/// fallbacks; the `*_esp32s3.c` and `*.S` files are the
/// PIE-accelerated specializations that give the ~7× speedup.
fn compile_esp_nn(esp_nn: &Path) {
    let mut b = s3_build();
    b.include(esp_nn.join("include"))
        .include(esp_nn.join("src/common"))
        // CONFIG_IDF_TARGET_ESP32S3 selects the S3 declarations in
        // esp_nn.h; CONFIG_NN_OPTIMIZED gates the S3-specific
        // overrides over the ANSI references.
        .define("CONFIG_IDF_TARGET_ESP32S3", "1")
        .define("CONFIG_NN_OPTIMIZED", "1")
        // Suppress the -Wunused-* warnings from compiling subsets
        // of esp-nn — the upstream code is fine but linting it
        // standalone surfaces parameters that are only used by
        // alternate code paths we don't pull in.
        .flag("-Wno-unused-parameter")
        .flag("-Wno-unused-function")
        // Convolution kernels — ANSI depthwise + ANSI conv
        // references, plus the S3-SIMD 1x1 pointwise. The 1x1
        // file is the one that exercises the PIE assembly route
        // (cf. the 59 `ee.*` opcode sanity check in the README);
        // the ANSI files are reference fallbacks that work on
        // every Xtensa core.
        .file(esp_nn.join("src/convolution/esp_nn_depthwise_conv_ansi.c"))
        .file(esp_nn.join("src/convolution/esp_nn_conv_ansi.c"))
        .file(esp_nn.join("src/convolution/esp_nn_conv_s8_1x1_esp32s3.c"))
        // Dense layer (final classifier head).
        .file(esp_nn.join("src/fully_connected/esp_nn_fully_connected_ansi.c"))
        // S3 assembly common helpers — referenced by the S3 C
        // kernels above.
        .file(esp_nn.join("src/common/esp_nn_dot_s8_esp32s3.S"))
        .file(esp_nn.join("src/common/esp_nn_multiply_by_quantized_mult_esp32s3.S"));
    b.compile("esp_nn");
}

/// Compile the TFLM port surface (system_setup, micro_log,
/// debug_log, micro_time) plus our C-ABI shim that links a
/// TFLM-internal symbol so dead-code-elimination can't strip the
/// port out of the staticlib.
fn compile_tflm_port_and_shim(tflm: &Path, manifest: &Path) {
    let mut b = s3_build();
    b.cpp(true)
        // C++17 — TFLM uses constexpr, structured bindings, and
        // template type-trait helpers that aren't in C++14. Use
        // unconditional `.flag()` rather than `.flag_if_supported`:
        // the latter silently skips the flag on any probe failure
        // (wrong sysroot, misdetected cross-compiler), which would
        // mask the real toolchain bug behind confusing syntax
        // errors in TFLM headers. `xtensa-esp32s3-elf-gcc` is GCC
        // 12+ and always accepts `-std=c++17`.
        .flag("-std=c++17")
        .include(tflm)
        .include(manifest.join("src"))
        // `TF_LITE_STRIP_ERROR_STRINGS` eliminates the
        // `DebugLog` and `DebugVsnprintf` bodies, dropping a
        // `<cstdio>` dep in the bargain. We can re-enable error
        // strings later by wiring them to defmt; for the
        // skeleton, the silence is fine.
        .define("TF_LITE_STRIP_ERROR_STRINGS", None)
        // Default `micro_time.cc` returns 0 unless
        // `TF_LITE_USE_CTIME` is defined. Leaving it 0 is the
        // bare-metal-correct choice; benchmarking comes later.
        .file(tflm.join("tensorflow/lite/micro/system_setup.cc"))
        .file(tflm.join("tensorflow/lite/micro/micro_log.cc"))
        .file(tflm.join("tensorflow/lite/micro/debug_log.cc"))
        .file(tflm.join("tensorflow/lite/micro/micro_time.cc"))
        .file(manifest.join("src/shim.cpp"));
    b.compile("esp_tflite_micro_shim");
}
