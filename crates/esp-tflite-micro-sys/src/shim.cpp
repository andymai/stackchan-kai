// Skeleton C-ABI shim. Calls `tflite::InitializeTarget()` so the
// linker has a reason to pull in the TFLM port objects we just
// compiled; without this, dead-code-elimination would strip them
// out of the staticlib and the build-system verification would
// be vacuous.
//
// The follow-up PR adds the real surface: a typed C-ABI wrapper
// around `tflite::MicroInterpreter` + `MicroMutableOpResolver`,
// then a Rust safe layer on top. For now the only function
// exported is enough to prove the build + link path.

#include "tensorflow/lite/micro/system_setup.h"

extern "C" void esp_tflite_micro_init() { tflite::InitializeTarget(); }
