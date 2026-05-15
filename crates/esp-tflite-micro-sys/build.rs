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
//! 2. TFLM library + 20 op kernels — the subset of `esp-tflite-micro`
//!    that constructs and runs a `MicroInterpreter` for microWakeWord
//!    inference. For the seven ops that have ESP-NN-accelerated
//!    variants (`add`, `conv`, `depthwise_conv`, `fully_connected`,
//!    `mul`, `pooling`, `softmax`), the upstream reference kernel is
//!    replaced by the `tensorflow/lite/micro/kernels/esp_nn/*.cc`
//!    variant; the remaining 13 ops use the upstream references.
//! 3. Shim — our C-ABI `extern "C"` wrapper. Hides the
//!    `MicroMutableOpResolver<N>` template behind opaque handles so
//!    bindgen can route Rust through a flat C-style interface.
//!
//! 4. Bindgen — runs over `src/shim.h` (pure C, no C++ template
//!    surface) and writes Rust FFI declarations to `OUT_DIR/bindings.rs`.

use std::env;
use std::fs;
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
    // and the TFLM op-kernel `esp_nn/*.cc` variants are the
    // consumers of esp-nn's kernels; esp-nn provides.
    //
    // Two separate `cc::Build`s because the include paths and
    // language standards differ — esp-nn is C/asm, TFLM is C++.
    compile_tflm_and_shim(&tflm, &esp_nn, &manifest);
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
    // `-mlongcalls` tells GAS to translate direct `call4`/`call8`
    // into indirect `callx` whenever the target is out of the
    // 1 MiB displacement window. Without it, a release build that
    // grows past ~1 MiB lands `dangerous relocation: call8: call
    // target out of range` errors on libgcc soft-float helpers
    // (`__divdf3`, `__extendsfdf2`, …) and libc (`memcpy`, `memset`,
    // `strcmp`). ESP-IDF sets `-mlongcalls` globally; the cc-rs
    // default doesn't, so mirroring upstream here is the fix.
    b.flag("-mlongcalls");
    b
}

/// Compile every esp-nn source relevant to ESP32-S3 — the `*_ansi.c`
/// reference kernels, every `*_esp32s3.{c,S}` PIE-accelerated
/// specialization, and the `*_opt.c` portable optimizations. Skip the
/// `*_esp32p4.*` variants (different SoC; instructions don't assemble
/// under the S3 toolchain). `-ffunction-sections` + `--gc-sections`
/// (already enabled in the workspace release profile) drop unreferenced
/// objects from the final binary, so over-including here costs build
/// time and archive size, not flash.
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
        .flag("-Wno-unused-function");

    add_esp_nn_sources(&mut b, &esp_nn.join("src"));
    b.compile("esp_nn");
}

/// Walk every category under `esp-nn/src/` and add the S3-relevant
/// kernel files to the build. The category list is the directory
/// layout esp-nn ships with (activation_functions, basic_math, common,
/// convolution, fully_connected, logistic, pooling, softmax).
fn add_esp_nn_sources(b: &mut cc::Build, src_root: &Path) {
    let categories = [
        "activation_functions",
        "basic_math",
        "common",
        "convolution",
        "fully_connected",
        "logistic",
        "pooling",
        "softmax",
    ];
    for category in categories {
        let dir = src_root.join(category);
        // Tell cargo to rerun this build script when the directory
        // listing changes. Without this, adding or removing an
        // upstream kernel file would silently produce a stale archive
        // (cargo only rebuilds when the explicitly-named files cc-rs
        // adds via `b.file()` change, not when new ones appear).
        println!("cargo:rerun-if-changed={}", dir.display());
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.unwrap().path();
            if is_s3_relevant(&path) {
                b.file(&path);
            }
        }
    }
}

/// Returns true for esp-nn source files that target ESP32-S3 or are
/// portable across Xtensa cores. The `*_esp32p4.*` files target a
/// different SoC and would fail at assembly time.
fn is_s3_relevant(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.contains("esp32p4") {
        return false;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    // Source-compilation units only. Header files in the same tree
    // (e.g. a hypothetical `esp_nn_conv_esp32s3.h`) would match the
    // `_esp32s3.` suffix below; passing one to `cc::Build::file` makes
    // GCC invoke `-c` on a header, which is implementation-defined.
    if !matches!(ext, "c" | "S") {
        return false;
    }
    // Suffix patterns rather than pure extensions: `_ansi.c` covers
    // the portable reference kernels, `_opt.c` covers the portable
    // optimized variants, and `_esp32s3.` (no leading underscore —
    // also catches `_s8_esp32s3.S`) covers the S3-specialized
    // assembly / C kernels.
    name.contains("_esp32s3.") || name.ends_with("_ansi.c") || name.ends_with("_opt.c")
}

/// Compile the TFLM library subset + the C-ABI shim. One `cc::Build`
/// — the shim's interpreter constructor needs every TFLM TU it
/// touches available in the same archive for the single-pass linker.
/// `esp_nn` is passed in for the include path; the seven
/// `kernels/esp_nn/*.cc` variants `#include <esp_nn.h>`.
fn compile_tflm_and_shim(tflm: &Path, esp_nn: &Path, manifest: &Path) {
    let mut b = s3_build();
    // Disable cc-rs's automatic `-lstdc++` link. Without it, the
    // toolchain's `libstdc++.a` gets pulled in and brings the full
    // C++ exception personality runtime
    // (`__gxx_personality_v0` → `_Unwind_GetIP` / `_Unwind_SetIP` /
    // …), plus thread-safe-statics machinery
    // (`pthread_mutex_lock` / `pthread_getspecific`), neither of
    // which can run under our build flags or runtime. `runtime_stubs.cpp`
    // provides the only C++ stdlib symbol TFLM actually needs
    // (`operator new` / `operator delete`); the link line below also
    // adds `-lm` for `frexp`, which `quantization_util.cc` uses.
    b.cpp_link_stdlib(None);
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
        // Local shim header sits next to its .cpp in `src/`. The
        // same directory holds our `<esp_timer.h>` stub — the
        // seven `kernels/esp_nn/*.cc` variants `#include`d that
        // ESP-IDF header for benchmarking, which we satisfy with
        // a header that returns 0.
        .include(manifest.join("src"))
        // esp-nn's public header — pulled in by the seven
        // `kernels/esp_nn/*.cc` variants for the
        // `esp_nn_{add,conv,...}_*` declarations they dispatch to.
        .include(esp_nn.join("include"))
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
    add_tflm_op_kernels(&mut b, tflm);

    // Our shim TU — bridges Rust over the templated TFLM classes.
    b.file(manifest.join("src/shim.cpp"));
    // Runtime stubs — `abort()` plus the minimal C++ runtime
    // (`operator new` / `operator delete`) that TFLM references
    // when we cut libstdc++ out of the link line. See
    // `runtime_stubs.cpp` for the full rationale.
    b.file(manifest.join("src/runtime_stubs.cpp"));

    b.compile("esp_tflite_micro");

    // `frexp` is the one libm function TFLM's `quantization_util.cc`
    // pulls in. Re-add libm on the link line — we cut the default
    // libstdc++ above, and the firmware crate's `-nodefaultlibs`
    // skips libm too. `=` (no `static=` / `dylib=`) lets the
    // toolchain pick whichever flavour it ships.
    println!("cargo:rustc-link-lib=m");
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

/// TFLM op-kernel `.cc` files. Mirrors upstream's
/// `file(GLOB srcs_kernels)` minus the seven ops that have ESP-NN
/// accelerated variants (`add`, `conv`, `depthwise_conv`,
/// `fully_connected`, `mul`, `pooling`, `softmax`) — those are
/// substituted with their `kernels/esp_nn/<op>.cc` counterparts.
/// Over-inclusion is intentional: `-ffunction-sections` +
/// `--gc-sections` drop unreferenced kernels from the final binary,
/// and tracking the per-microWakeWord-op transitive shared-code
/// surface (`*_common.cc`, dispatch tables, helper utilities) by
/// hand drifts immediately when upstream re-shuffles internals.
fn add_tflm_op_kernels(b: &mut cc::Build, tflm: &Path) {
    let kernels = tflm.join("tensorflow/lite/micro/kernels");

    // Tell cargo to rerun this build script when the directory
    // listing changes. Without this, adding or removing a kernel
    // file under `tensorflow/lite/micro/kernels/` would silently
    // produce a stale archive.
    println!("cargo:rerun-if-changed={}", kernels.display());

    // The seven ops whose reference `.cc` we skip — the ESP-NN
    // variant below provides `Register_*` symbols with the same
    // names and linking both would be a multiple-definition error.
    let esp_nn_replacements: &[&str] = &[
        "add.cc",
        "conv.cc",
        "depthwise_conv.cc",
        "fully_connected.cc",
        "mul.cc",
        "pooling.cc",
        "softmax.cc",
    ];

    let kernel_entries =
        fs::read_dir(&kernels).unwrap_or_else(|e| panic!("read_dir {}: {e}", kernels.display()));
    for entry in kernel_entries {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !matches!(ext, "cc" | "c") {
            continue;
        }
        if esp_nn_replacements.contains(&name) {
            continue;
        }
        b.file(&path);
    }

    // ESP-NN-accelerated variants — replace the seven skipped above.
    let esp_nn = kernels.join("esp_nn");
    for f in esp_nn_replacements {
        b.file(esp_nn.join(f));
    }
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
