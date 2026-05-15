// Bare-metal runtime stubs for the few C / C++ symbols TFLM references
// when we cut libstdc++ out of the link line and run on `-nodefaultlibs`.
//
// Compilation flags in `build.rs` set `-fno-exceptions` and
// `-fno-rtti`; `cpp_link_stdlib(None)` keeps cc-rs from auto-linking
// `libstdc++.a`. That leaves three classes of unresolved symbol:
//
// 1. `abort()` — `TFLITE_DCHECK_*` macros expand to it. Real
//    triggering would be a programmer bug in TFLM; nothing on the
//    detection-path can hit it under normal operation.
// 2. `operator new` / `operator delete` — TFLM uses these for shim-
//    side internal control structures (the interpreter's per-op
//    state allocators). Routes through the firmware's `esp-alloc`-
//    backed global allocator via Rust-side `extern "C"` helpers so
//    everything lands in the same heap.
// 3. C++ ABI guards for static-local init (`__cxa_guard_acquire` /
//    `__cxa_guard_release`). TFLM has a few constexpr-shaped static
//    locals that the compiler conservatively wraps in guards even
//    though they're trivially constructible. Single-threaded sema
//    suffices.

#include <cstddef>
#include <new>

// The `std::nothrow` global tag object. With libstdc++ pulled out of
// the link, the C++ ABI's well-known instance is unresolved; provide
// it ourselves. `nothrow_t` is an empty struct so the storage is
// zero-sized and the address is the only thing that matters.
namespace std {
const nothrow_t nothrow{};
}  // namespace std

extern "C" {

// Rust-side allocator bridge — implemented in `lib.rs`. Lets the C++
// `operator new` route through `esp-alloc` so PSRAM is the backing
// store for both Rust `Vec` and C++ `new`. Without this, we'd need
// to give TFLM its own arena, which would fragment heap usage.
void* etms_runtime_alloc(std::size_t size);
void etms_runtime_free(void* ptr);

// Replace newlib's abort. `noreturn` keeps GCC from flagging code that
// follows a call as unreachable-warning material; `noinline` keeps the
// stub out of TFLM's hot paths if the optimiser ever decides to inline.
[[noreturn]] __attribute__((noinline)) void abort() {
  while (true) {
    __asm__ volatile("nop");
  }
}

// C++ ABI guards for thread-safe static-local initialization. We are
// single-threaded with respect to TFLM construction (the wake-word
// task constructs the interpreter once at boot before any other code
// can race), so a no-op suffices. `__cxa_guard_acquire` returns
// non-zero to indicate "uninitialized — please initialize"; we cache
// the guard variable's first byte so subsequent acquisitions skip
// initialization.
int __cxa_guard_acquire(char* guard) {
  if (*guard) {
    return 0;
  }
  return 1;
}

void __cxa_guard_release(char* guard) { *guard = 1; }

void __cxa_guard_abort(char* /*guard*/) {
  // No-op: an abort path during static-local init can't happen
  // without exceptions enabled.
}

// Pure-virtual call landing pad. The compiler emits references to
// `__cxa_pure_virtual` in vtables for abstract classes; reaching one
// at runtime would indicate a TFLM bug.
[[noreturn]] void __cxa_pure_virtual() { abort(); }

}  // extern "C"

// Global C++ allocation operators. TFLM with `-fno-exceptions` never
// throws `std::bad_alloc`, so the `nothrow` and throwing forms can
// share an implementation. All four overloads route through the same
// Rust-side bridge so heap accounting is unified.

void* operator new(std::size_t size) { return etms_runtime_alloc(size); }
void* operator new[](std::size_t size) { return etms_runtime_alloc(size); }
void operator delete(void* ptr) noexcept { etms_runtime_free(ptr); }
void operator delete[](void* ptr) noexcept { etms_runtime_free(ptr); }
// Sized-delete overloads (C++14). The size hint is informational; we
// ignore it because the underlying allocator already tracks block
// sizes.
void operator delete(void* ptr, std::size_t /*size*/) noexcept {
  etms_runtime_free(ptr);
}
void operator delete[](void* ptr, std::size_t /*size*/) noexcept {
  etms_runtime_free(ptr);
}

// `new (std::nothrow) T` overloads — what the shim's
// `ResolverImpl` / `InterpreterImpl` constructors actually call.
// Allocation failure returns nullptr, matching the nothrow contract.
void* operator new(std::size_t size, const std::nothrow_t&) noexcept {
  return etms_runtime_alloc(size);
}
void* operator new[](std::size_t size, const std::nothrow_t&) noexcept {
  return etms_runtime_alloc(size);
}
void operator delete(void* ptr, const std::nothrow_t&) noexcept {
  etms_runtime_free(ptr);
}
void operator delete[](void* ptr, const std::nothrow_t&) noexcept {
  etms_runtime_free(ptr);
}
