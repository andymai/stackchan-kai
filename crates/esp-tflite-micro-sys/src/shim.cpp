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

// Resolves an interpreter handle to the underlying `MicroInterpreter`
// pointer. Returns nullptr if `interpreter` is null; callers must
// handle that case. The cast back to a mutable pointer is safe even
// when called from `const EtmsInterpreter*` shim wrappers — TFLM's
// `input()` / `output()` / `inputs_size()` accessors are non-const
// for historical reasons but don't mutate the interpreter for the
// queries this shim makes.
inline tflite::MicroInterpreter* as_interp(const EtmsInterpreter* interpreter) {
  if (interpreter == nullptr) {
    return nullptr;
  }
  return &const_cast<InterpreterImpl*>(
              reinterpret_cast<const InterpreterImpl*>(interpreter))
              ->interpreter;
}

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

// One-liner wrapper per op-registration call. The expansion is
// uniform: cast the opaque handle back to `ResolverImpl*` and call
// the matching `Add*` method, surfacing the `TfLiteStatus` as int.
//
// Naming convention: snake_case in the C ABI to match the rest of
// the shim; CamelCase on the C++ side to match TFLM's `Add*`
// methods.
#define ETMS_RESOLVER_ADD_OP(snake_name, MethodName)                       \
  int etms_resolver_add_##snake_name(EtmsResolver* resolver) {             \
    if (resolver == nullptr) {                                             \
      return kTfLiteError;                                                 \
    }                                                                      \
    return reinterpret_cast<ResolverImpl*>(resolver)->MethodName();        \
  }

ETMS_RESOLVER_ADD_OP(add, AddAdd)
ETMS_RESOLVER_ADD_OP(assign_variable, AddAssignVariable)
ETMS_RESOLVER_ADD_OP(average_pool_2d, AddAveragePool2D)
ETMS_RESOLVER_ADD_OP(call_once, AddCallOnce)
ETMS_RESOLVER_ADD_OP(concatenation, AddConcatenation)
ETMS_RESOLVER_ADD_OP(conv_2d, AddConv2D)
ETMS_RESOLVER_ADD_OP(depthwise_conv_2d, AddDepthwiseConv2D)
ETMS_RESOLVER_ADD_OP(fully_connected, AddFullyConnected)
ETMS_RESOLVER_ADD_OP(logistic, AddLogistic)
ETMS_RESOLVER_ADD_OP(max_pool_2d, AddMaxPool2D)
ETMS_RESOLVER_ADD_OP(mean, AddMean)
ETMS_RESOLVER_ADD_OP(mul, AddMul)
ETMS_RESOLVER_ADD_OP(pack, AddPack)
ETMS_RESOLVER_ADD_OP(pad, AddPad)
ETMS_RESOLVER_ADD_OP(quantize, AddQuantize)
ETMS_RESOLVER_ADD_OP(read_variable, AddReadVariable)
ETMS_RESOLVER_ADD_OP(reshape, AddReshape)
ETMS_RESOLVER_ADD_OP(split_v, AddSplitV)
ETMS_RESOLVER_ADD_OP(strided_slice, AddStridedSlice)
ETMS_RESOLVER_ADD_OP(var_handle, AddVarHandle)

#undef ETMS_RESOLVER_ADD_OP

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

size_t etms_interpreter_inputs_size(const EtmsInterpreter* interpreter) {
  auto* impl = as_interp(interpreter);
  return impl == nullptr ? 0 : impl->inputs_size();
}

size_t etms_interpreter_outputs_size(const EtmsInterpreter* interpreter) {
  auto* impl = as_interp(interpreter);
  return impl == nullptr ? 0 : impl->outputs_size();
}

uint8_t* etms_interpreter_input_data(EtmsInterpreter* interpreter, size_t idx) {
  auto* impl = as_interp(interpreter);
  if (impl == nullptr || idx >= impl->inputs_size()) {
    return nullptr;
  }
  TfLiteTensor* t = impl->input(idx);
  return t == nullptr ? nullptr : reinterpret_cast<uint8_t*>(t->data.data);
}

size_t etms_interpreter_input_bytes(const EtmsInterpreter* interpreter, size_t idx) {
  auto* impl = as_interp(interpreter);
  if (impl == nullptr || idx >= impl->inputs_size()) {
    return 0;
  }
  const TfLiteTensor* t = impl->input(idx);
  return t == nullptr ? 0 : t->bytes;
}

const uint8_t* etms_interpreter_output_data(const EtmsInterpreter* interpreter, size_t idx) {
  auto* impl = as_interp(interpreter);
  if (impl == nullptr || idx >= impl->outputs_size()) {
    return nullptr;
  }
  const TfLiteTensor* t = impl->output(idx);
  return t == nullptr ? nullptr : reinterpret_cast<const uint8_t*>(t->data.data);
}

size_t etms_interpreter_output_bytes(const EtmsInterpreter* interpreter, size_t idx) {
  auto* impl = as_interp(interpreter);
  if (impl == nullptr || idx >= impl->outputs_size()) {
    return 0;
  }
  const TfLiteTensor* t = impl->output(idx);
  return t == nullptr ? 0 : t->bytes;
}

}  // extern "C"
