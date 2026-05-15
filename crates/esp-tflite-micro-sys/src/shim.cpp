// C-ABI shim — see `shim.h` for the contract.

#include "shim.h"

#include <new>

#include "tensorflow/lite/micro/micro_interpreter.h"
#include "tensorflow/lite/micro/micro_mutable_op_resolver.h"
#include "tensorflow/lite/micro/system_setup.h"
#include "tensorflow/lite/schema/schema_generated.h"

namespace {

// microWakeWord's operator set is 20 ops (CallOnce, VarHandle, Reshape,
// ReadVariable, StridedSlice, Concatenation, AssignVariable, Conv2D,
// Mul, Add, Mean, FullyConnected, Logistic, Quantize, DepthwiseConv2D,
// AveragePool2D, MaxPool2D, Pad, Pack, SplitV). Fixing the template
// parameter at this number means the resolver size is constant across
// the build whether or not all ops have been wired yet.
constexpr unsigned int kResolverCapacity = 20;

using ResolverImpl = tflite::MicroMutableOpResolver<kResolverCapacity>;

// `EtmsInterpreter` owns the `MicroInterpreter` and keeps the model
// pointer alongside it so the destroyer can reason about lifetime in
// one place. The model bytes themselves are caller-owned; we just
// borrow the parsed `Model*` flatbuffer view.
struct InterpreterImpl {
  const tflite::Model* model;
  tflite::MicroInterpreter interpreter;

  InterpreterImpl(const tflite::Model* m, const tflite::MicroOpResolver& r,
                  uint8_t* arena, size_t arena_size)
      : model(m), interpreter(m, r, arena, arena_size) {}
};

}  // namespace

extern "C" {

void etms_init(void) { tflite::InitializeTarget(); }

EtmsResolver* etms_resolver_create(void) {
  // Placement-new into a heap-allocated buffer — the default
  // operator-new on this build is `esp-alloc`'s PSRAM-backed heap.
  // Cast to the opaque tag struct type the header advertises.
  return reinterpret_cast<EtmsResolver*>(new (std::nothrow) ResolverImpl());
}

void etms_resolver_destroy(EtmsResolver* resolver) {
  delete reinterpret_cast<ResolverImpl*>(resolver);
}

EtmsInterpreter* etms_interpreter_create(const uint8_t* model_bytes,
                                         size_t model_len,
                                         const EtmsResolver* resolver,
                                         uint8_t* arena,
                                         size_t arena_len) {
  if (model_bytes == nullptr || resolver == nullptr || arena == nullptr) {
    return nullptr;
  }
  // `model_len` is recorded for the future verifier hook; TFLM's
  // `GetModel` itself reads the flatbuffer length out of the buffer
  // header. Suppress the unused-parameter warning until then.
  (void)model_len;

  const tflite::Model* model = tflite::GetModel(model_bytes);
  if (model == nullptr) {
    return nullptr;
  }
  if (model->version() != TFLITE_SCHEMA_VERSION) {
    return nullptr;
  }

  const auto* resolver_impl =
      reinterpret_cast<const ResolverImpl*>(resolver);
  auto* impl = new (std::nothrow)
      InterpreterImpl(model, *resolver_impl, arena, arena_len);
  return reinterpret_cast<EtmsInterpreter*>(impl);
}

void etms_interpreter_destroy(EtmsInterpreter* interpreter) {
  delete reinterpret_cast<InterpreterImpl*>(interpreter);
}

int etms_interpreter_allocate_tensors(EtmsInterpreter* interpreter) {
  if (interpreter == nullptr) {
    return kTfLiteError;
  }
  return reinterpret_cast<InterpreterImpl*>(interpreter)
      ->interpreter.AllocateTensors();
}

int etms_interpreter_invoke(EtmsInterpreter* interpreter) {
  if (interpreter == nullptr) {
    return kTfLiteError;
  }
  return reinterpret_cast<InterpreterImpl*>(interpreter)->interpreter.Invoke();
}

}  // extern "C"
