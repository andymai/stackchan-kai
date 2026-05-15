# esp-tflite-micro-sys

FFI bindings to [`esp-tflite-micro`] + [`esp-nn`] for ESP32-S3
keyword-spotting inference. Wake-word inference is the single
deliberate exception to the firmware crate's "no C/C++ in the
binary" non-goal, scoped to one synchronous `Invoke()` call from
an embassy task.

## Status — foundation slice

This skeleton crate ships the build plumbing and verifies the
toolchain path end-to-end:

- vendored `esp-nn` sources compile via `cc-rs` with the
  ESP32-S3-specific gcc binary (`xtensa-esp32s3-elf-gcc`), the
  ESP-DSP / PIE extension assembly assembles cleanly, and 59
  `ee.*` SIMD opcodes show up in the resulting object file;
- vendored TFLM port surface (`system_setup` + `micro_log` +
  `debug_log` + `micro_time`) compiles bare-metal with zero
  FreeRTOS or ESP-IDF dependencies, gated by
  `-DTF_LITE_STRIP_ERROR_STRINGS` to drop the `<cstdio>` dep;
- one C-ABI shim function (`esp_tflite_micro_init`) links
  against `tflite::InitializeTarget()` so dead-code-elimination
  can't strip the port-surface symbols out of the staticlib;
- the firmware crate can depend on this with no impact on the
  host CI gates (the workspace's `just check` / `just ci` /
  `just msrv` recipes exclude this crate alongside
  `stackchan-firmware`).

The interpreter surface — `MicroInterpreter`,
`MicroMutableOpResolver`, the operator kernels needed for
`hey_jarvis` microWakeWord (`Conv2D`, `DepthwiseConv2D`,
`FullyConnected`, `MaxPool2D`, `StridedSlice`, `Concatenation`,
`Pack`, `SplitV`, plus the resource-variable streaming
machinery: `VarHandle`, `ReadVariable`, `AssignVariable`) — and
the safe Rust wrapper around it land in a follow-up PR. The
research grounding the design decision lives in
`.notes/arc-4b-inference-research.md` (gitignored).

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

[`esp-tflite-micro`]: https://github.com/espressif/esp-tflite-micro
[`esp-nn`]: https://github.com/espressif/esp-nn
[espressif/esp-nn]: https://github.com/espressif/esp-nn
[espressif/esp-tflite-micro]: https://github.com/espressif/esp-tflite-micro
