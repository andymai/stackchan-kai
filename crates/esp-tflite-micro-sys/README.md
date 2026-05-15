# esp-tflite-micro-sys

FFI bindings to [`esp-tflite-micro`] + [`esp-nn`] for ESP32-S3
keyword-spotting inference. Wake-word inference is the single
deliberate exception to the firmware crate's "no C/C++ in the
binary" non-goal, scoped to one synchronous `Invoke()` call from
an embassy task.

## Surface

Two owned Rust types front the templated TFLM classes:

- `Resolver` wraps `tflite::MicroMutableOpResolver<20>`. The
  template parameter is fixed in the C-ABI shim at the operator
  count microWakeWord uses, so the same resolver size carries
  through to the production wake-word task. Twenty `add_*`
  methods register the microWakeWord operator set
  (`add_conv_2d`, `add_depthwise_conv_2d`, `add_fully_connected`,
  `add_var_handle`, `add_read_variable`, `add_assign_variable`,
  `add_strided_slice`, `add_concatenation`, `add_pack`,
  `add_split_v`, `add_call_once`, `add_reshape`,
  `add_average_pool_2d`, `add_max_pool_2d`, `add_logistic`,
  `add_quantize`, `add_pad`, `add_mean`, `add_add`, `add_mul`);
  the seven ops with ESP-NN-accelerated variants dispatch into
  esp-nn's hand-tuned LX7 SIMD kernels at registration time.
- `Interpreter<'a>` wraps `tflite::MicroInterpreter`. It borrows
  the model bytes, the resolver, and the tensor arena for its
  lifetime — the underlying C++ object holds raw pointers into
  all three.

`Interpreter::allocate_tensors` runs TFLM's memory planner over
the caller-supplied arena; `Interpreter::invoke` runs one
inference pass. Both return `Result<(), TfLiteStatus>` where the
non-zero `TfLiteStatus` codes are surfaced verbatim.

Tensor I/O accessors (`Interpreter::input(idx)`, `output(idx)`,
typed `[u8]` / `[i8]` slice views) wire up alongside the first
firmware-side consumer.

The C-ABI shim header (`src/shim.h`) is pure C — `bindgen` runs
against it without touching the C++ template surface; the
generated declarations end up in `OUT_DIR/bindings.rs` and
re-export under the crate's private `ffi` module.

## Build host

`xtensa-esp32s3-none-elf` only. Host builds intentionally fail —
the crate has nothing useful to expose on x86_64 and pretending
otherwise would push the failure to wake-word task spawn time.

## Vendored upstreams

| Library | Upstream | Commit |
|---------|----------|--------|
| esp-nn  | [espressif/esp-nn] | see `vendor/COMMITS.txt` |
| esp-tflite-micro | [espressif/esp-tflite-micro] | see `vendor/COMMITS.txt` |

Both Apache-2.0; LICENSE files preserved in each vendored tree.
The TFLM tree includes its header-only third-party deps
(`flatbuffers`, `gemmlowp`, `ruy`) under their respective licenses,
also Apache-2.0.

## Build-system gotchas worth remembering

The `esp` Rust toolchain wires `cc-rs`'s xtensa target detection
to the **generic** `xtensa-esp-elf-gcc`, which does not
recognize the ESP32-S3 DSP / PIE extension instructions and
errors at assembly time with "unknown opcode or format name
'ee.vldbc.16'". The `esp` toolchain ships an S3-specific gcc in
the same directory; pin it via
`cc::Build::compiler("xtensa-esp32s3-elf-gcc")` and the SIMD
path assembles cleanly. The 59-opcode count in the output
`esp_nn_conv_s8_1x1_esp32s3.o` is the sanity check that
distinguishes a working build from a "compiles but fell back to
slow path" build.

Static-library link order matters. `cc-rs` emits
`cargo:rustc-link-lib=static=…` in call order; GNU ld is
single-pass and only pulls an archive member if a reference has
already been seen. The shim/TFLM archive precedes esp-nn on the
link line — the shim is the consumer of esp-nn's kernels
(transitively, once op kernels are wired), esp-nn provides.

`-fno-rtti` and `-fno-exceptions` match upstream
`esp-tflite-micro`'s compile flags. The shim uses
`new (std::nothrow)` so allocation failures don't need exceptions
to propagate.

[`esp-tflite-micro`]: https://github.com/espressif/esp-tflite-micro
[`esp-nn`]: https://github.com/espressif/esp-nn
[espressif/esp-nn]: https://github.com/espressif/esp-nn
[espressif/esp-tflite-micro]: https://github.com/espressif/esp-tflite-micro
