// Per-file allow: build.rs documents protocol-spec terms (TFLM,
// ESP-NN, ESP-DSP / PIE, FreeRTOS, ESP-IDF, MixConv, MicroInterpreter,
// MicroMutableOpResolver, microWakeWord) in nearly every sentence;
// backticking each occurrence would drown the prose. Same pattern as
// `stackchan-net::blufi`'s module-level allow.
#![allow(clippy::doc_markdown)]
// Build scripts run on the host and the cargo contract is "panic on
// error so cargo reports it as a build failure." `unwrap_used` /
// `expect_used` / `panic` are the right defaults for runtime Rust
// but not for build.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Build script for `esp-tflite-micro-sys`.
//!
//! Three compilation passes plus a bindgen pass:
//!
//! 1. ESP-NN — Espressif's int8 kernel library, ESP32-S3 PIE-accelerated.
//!    The `xtensa-esp32s3-elf-gcc` pin is required because the generic
//!    `xtensa-esp-elf-gcc` doesn't recognise the S3's DSP / PIE
//!    extension instructions.
//! 2. TFLM library — the subset of `esp-tflite-micro` that's needed
//!    to construct a `MicroInterpreter`. Op kernels are not included
//!    in this slice; the resolver template is instantiated at size 20
//!    but `MicroMutableOpResolver::Add*` calls are wired separately.
//! 3. Shim — our C-ABI `extern "C"` wrapper. Hides the
//!    `MicroMutableOpResolver<N>` template behind opaque handles so
//!    bindgen can route Rust through a flat C-style interface.
//!
//! 4. Bindgen — runs over `src/shim.h` (pure C, no C++ template
//!    surface) and writes Rust FFI declarations to `OUT_DIR/bindings.rs`.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vendor = manifest.join("vendor");
    let esp_nn = vendor.join("esp-nn");
    let tflm = vendor.join("esp-tflite-micro");

    // Shim + TFLM library archive comes first on the link line,
    // esp-nn second. cc-rs emits `cargo:rustc-link-lib=static=…`
    // in call order; GNU ld is single-pass and only pulls an
    // archive member if a reference has already been seen, so
    // the consumer archive must precede the provider. The shim
    // is the consumer of esp-nn's kernels (transitively, once op
    // kernels are wired); esp-nn provides.
    //
    // Two separate `cc::Build`s because the include paths and
    // language standards differ — esp-nn is C/asm, TFLM is C++.
    compile_tflm_and_shim(&tflm, &manifest);
    compile_esp_nn(&esp_nn);

    generate_bindings(&manifest);
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

/// Compile the TFLM library subset + the C-ABI shim. One `cc::Build`
/// — the shim's interpreter constructor needs every TFLM TU it
/// touches available in the same archive for the single-pass linker.
fn compile_tflm_and_shim(tflm: &Path, manifest: &Path) {
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
        // Match upstream esp-tflite-micro's compile flags. RTTI
        // and exceptions are disabled to shrink the binary; TFLM
        // is designed to work without either, and our shim uses
        // `new (std::nothrow)` so allocation failures don't need
        // exceptions to propagate.
        .flag("-fno-rtti")
        .flag("-fno-exceptions")
        // Upstream mute list (see esp-tflite-micro `CMakeLists.txt`).
        // TFLM's vendored sources have a few diagnostics that the
        // upstream build promotes to errors-or-warnings selectively;
        // we mirror the same set so vendoring stays inert.
        .flag("-Wno-unused-parameter")
        .flag("-Wno-sign-compare")
        .flag("-Wno-double-promotion")
        .flag("-Wno-missing-field-initializers")
        .flag("-Wno-shadow")
        .flag("-Wno-type-limits")
        // Include paths mirror upstream's `idf_component_register`.
        // `.` (tflm root) is what makes `<tensorflow/lite/...>`
        // resolve; the third_party trees are header-only deps.
        .include(tflm)
        .include(tflm.join("third_party/flatbuffers/include"))
        .include(tflm.join("third_party/gemmlowp"))
        .include(tflm.join("third_party/ruy"))
        // Local shim header sits next to its .cpp in `src/`.
        .include(manifest.join("src"))
        // `TF_LITE_STATIC_MEMORY` disables runtime allocator paths
        // we don't use; `TF_LITE_DISABLE_X86_NEON` is harmless on
        // xtensa but matches upstream; `TF_LITE_STRIP_ERROR_STRINGS`
        // eliminates `DebugLog`/`DebugVsnprintf` bodies (drops a
        // `<cstdio>` dep in the bargain); `ESP_NN` activates the
        // ESP-NN-accelerated kernels in the TFLM op layer.
        .define("TF_LITE_STATIC_MEMORY", None)
        .define("TF_LITE_DISABLE_X86_NEON", None)
        .define("TF_LITE_STRIP_ERROR_STRINGS", None)
        .define("ESP_NN", None);

    add_tflm_library_sources(&mut b, tflm);

    // Our shim TU — bridges Rust over the templated TFLM classes.
    b.file(manifest.join("src/shim.cpp"));

    b.compile("esp_tflite_micro");
}

/// Lift upstream's `lib_srcs` from `esp-tflite-micro/CMakeLists.txt`
/// minus the op kernels (`tensorflow/lite/micro/kernels/*.cc`) — those
/// land alongside the resolver `Add*` calls in a subsequent slice. The
/// list here is what's transitively needed to construct, destruct, and
/// run `AllocateTensors`/`Invoke` on a `MicroInterpreter`.
fn add_tflm_library_sources(b: &mut cc::Build, tflm: &Path) {
    let micro = tflm.join("tensorflow/lite/micro");
    let micro_files = [
        // Note: debug_log, micro_log, micro_time, system_setup are
        // already part of the port layer below.
        "flatbuffer_utils.cc",
        "memory_helpers.cc",
        "micro_allocation_info.cc",
        "micro_allocator.cc",
        "micro_context.cc",
        "micro_interpreter.cc",
        "micro_interpreter_context.cc",
        "micro_interpreter_graph.cc",
        "micro_op_resolver.cc",
        "micro_profiler.cc",
        "micro_resource_variable.cc",
        "micro_utils.cc",
        "recording_micro_allocator.cc",
    ];
    for f in micro_files {
        b.file(micro.join(f));
    }

    // Port surface — TFLM's HAL hooks. Bare-metal `micro_time.cc`
    // returns 0 unless `TF_LITE_USE_CTIME` is defined; the
    // upstream-esp variant pulls `<esp_timer.h>` which we don't
    // want for the no_std-without-ESP-IDF firmware. `debug_log` /
    // `micro_log` bodies vanish under `TF_LITE_STRIP_ERROR_STRINGS`.
    for f in [
        "debug_log.cc",
        "micro_log.cc",
        "micro_time.cc",
        "system_setup.cc",
    ] {
        b.file(micro.join(f));
    }

    // Arena + memory planner — bring-your-own-buffer allocators
    // over a caller-supplied `uint8_t*` tensor arena.
    let arena = micro.join("arena_allocator");
    for f in [
        "non_persistent_arena_buffer_allocator.cc",
        "persistent_arena_buffer_allocator.cc",
        "recording_single_arena_buffer_allocator.cc",
        "single_arena_buffer_allocator.cc",
    ] {
        b.file(arena.join(f));
    }

    let planner = micro.join("memory_planner");
    for f in ["greedy_memory_planner.cc", "linear_memory_planner.cc"] {
        b.file(planner.join(f));
    }

    // tflite_bridge — TFLM's adapter from its internal types to
    // the public `TfLite*` C ABI used by op signatures.
    let bridge = micro.join("tflite_bridge");
    for f in [
        "flatbuffer_conversions_bridge.cc",
        "micro_error_reporter.cc",
    ] {
        b.file(bridge.join(f));
    }

    // Core C ABI types + flatbuffer plumbing.
    let core = tflm.join("tensorflow/lite/core");
    b.file(core.join("c/common.cc"))
        .file(core.join("api/flatbuffer_conversions.cc"))
        .file(core.join("api/tensor_utils.cc"));

    // Reference internals — int8 quantization arithmetic, tensor
    // utility helpers, common comparison kernels. The internal/
    // tree is reference C++, no SIMD; the SIMD substitution
    // happens inside esp-nn kernels via the `ESP_NN` define.
    let internal = tflm.join("tensorflow/lite/kernels/internal");
    for f in [
        "common.cc",
        "portable_tensor_utils.cc",
        "quantization_util.cc",
        "tensor_ctypes.cc",
        "tensor_utils.cc",
    ] {
        b.file(internal.join(f));
    }

    b.file(internal.join("reference/comparisons.cc"))
        .file(internal.join("reference/portable_tensor_utils.cc"));

    b.file(tflm.join("tensorflow/lite/kernels/kernel_util.cc"));

    // MLIR-side schema utilities + error reporter — TFLM imports
    // these out of the TensorFlow MLIR tree (not the lite tree)
    // for historical reasons; both upstream and we pull them in
    // verbatim.
    let mlir = tflm.join("tensorflow/compiler/mlir/lite");
    b.file(mlir.join("core/api/error_reporter.cc"))
        .file(mlir.join("schema/schema_utils.cc"));
}

/// Run bindgen over the C-ABI shim header so Rust callers can use
/// `extern "C"` declarations without writing them by hand. The header
/// is pure C and uses only `<stddef.h>` + `<stdint.h>`, so libclang
/// can parse it against the host system's headers regardless of cross-
/// compilation target.
fn generate_bindings(manifest: &Path) {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    let header = manifest.join("src/shim.h");

    println!("cargo:rerun-if-changed={}", header.display());

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        // `core::ffi` instead of `std::ffi` for `no_std`.
        .use_core()
        .ctypes_prefix("core::ffi")
        // Skip the `_test_layout_*` functions bindgen normally
        // emits — they expand to constants the linker can't drop
        // without LTO and they're only meaningful when running
        // host tests against generated C types, which we don't.
        .layout_tests(false)
        // The header tags its opaque handle types as
        // `typedef struct EtmsResolver EtmsResolver` etc.; tell
        // bindgen to treat them as opaque rather than introspecting
        // their (zero-sized, incomplete) `struct` bodies.
        .allowlist_type("Etms.*")
        .allowlist_function("etms_.*")
        .generate()
        .expect("bindgen failed for src/shim.h");

    bindings
        .write_to_file(&out)
        .unwrap_or_else(|e| panic!("write bindings.rs ({}): {e}", out.display()));
}
