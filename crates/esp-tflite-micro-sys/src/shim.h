// C-ABI shim over the templated TFLM classes (`MicroMutableOpResolver<N>`,
// `MicroInterpreter`). bindgen handles flat `extern "C"` declarations
// cleanly but chokes on C++ templates and namespaces; routing every
// Rust caller through this header keeps the FFI surface inside what
// bindgen can grok.
//
// Two opaque handle types:
//
// - `EtmsResolver` wraps `MicroMutableOpResolver<20>`. The template
//   parameter is the upper bound on registered ops; 20 matches the
//   microWakeWord operator set so the same resolver size carries
//   through to the production wake-word task. Resolvers are reusable
//   across interpreter instances.
// - `EtmsInterpreter` wraps a `MicroInterpreter` plus the model
//   pointer it borrows (the caller-owned model bytes must outlive
//   the handle; documented on `etms_interpreter_create`).
//
// All functions return a `TfLiteStatus` int (0 = ok, non-zero = error)
// where applicable. Lifecycle functions return `NULL` on construction
// failure (bad model, allocation failure, etc.).

#ifndef ESP_TFLITE_MICRO_SYS_SHIM_H_
#define ESP_TFLITE_MICRO_SYS_SHIM_H_

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct EtmsResolver EtmsResolver;
typedef struct EtmsInterpreter EtmsInterpreter;

// One-time TFLM port-layer init. Idempotent; the default
// `tflite::InitializeTarget()` is `{}` on this port. Kept exposed so
// dead-code-elimination can't strip the port-surface symbols.
void etms_init(void);

// --- Resolver lifecycle ----------------------------------------------------

// Allocates an empty `MicroMutableOpResolver<20>`. Returns NULL on
// allocation failure.
EtmsResolver* etms_resolver_create(void);

// Frees a resolver previously returned by `etms_resolver_create`.
// `NULL` is a no-op.
void etms_resolver_destroy(EtmsResolver* resolver);

// --- Interpreter lifecycle -------------------------------------------------

// Constructs a `MicroInterpreter` over the given model. The caller
// owns the model bytes, the resolver, and the arena, and must keep
// all three alive at least as long as the returned handle. The model
// bytes must remain valid (and the resolver and arena unmodified) for
// the entire interpreter lifetime — TFLM borrows pointers into them.
// Returns NULL on allocation failure or invalid model.
EtmsInterpreter* etms_interpreter_create(
    const uint8_t* model_bytes, size_t model_len,
    const EtmsResolver* resolver,
    uint8_t* arena, size_t arena_len);

// Frees an interpreter previously returned by `etms_interpreter_create`.
// `NULL` is a no-op.
void etms_interpreter_destroy(EtmsInterpreter* interpreter);

// Runs the planner pass and allocates input/output/intermediate
// tensors inside the arena. Must be called before `etms_interpreter_invoke`.
// Returns 0 (`kTfLiteOk`) on success.
int etms_interpreter_allocate_tensors(EtmsInterpreter* interpreter);

// Runs one inference pass. Inputs must have been written through the
// tensor pointers obtained from `etms_interpreter_input_*` since the
// last call. Returns 0 (`kTfLiteOk`) on success.
int etms_interpreter_invoke(EtmsInterpreter* interpreter);

#ifdef __cplusplus
}
#endif

#endif  // ESP_TFLITE_MICRO_SYS_SHIM_H_
