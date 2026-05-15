//! Safe Rust wrapper over `esp-tflite-micro` + `esp-nn` for
//! ESP32-S3 keyword-spotting inference.
//!
//! ## Surface
//!
//! Two owned types front the C++ TFLM classes:
//!
//! - [`Resolver`] wraps `tflite::MicroMutableOpResolver<20>`. The
//!   template parameter is fixed in the C-ABI shim at the operator
//!   count microWakeWord uses, so a single resolver is reusable
//!   across model variants without resizing.
//! - [`Interpreter`] wraps `tflite::MicroInterpreter`. It borrows
//!   the model bytes, the resolver, and the tensor arena for its
//!   lifetime — the underlying C++ object holds raw pointers into
//!   all three.
//!
//! Op registration (`Resolver::add_*`) and tensor I/O accessors
//! (`Interpreter::input`, `Interpreter::output`) are not yet wired —
//! once added, the interpreter is end-to-end usable from a Rust
//! `embassy` task.
//!
//! ## Why this exists
//!
//! Wake-word inference is the one deliberate exception to the
//! firmware crate's "no C/C++ in the binary" non-goal. Three
//! pure-Rust paths were evaluated against the published
//! microWakeWord operator set (`microflow`, `candle-core`,
//! custom MixConv interpreter); all three either lack the
//! resource-variable streaming machinery the model requires
//! (`microflow`: 8 of 20 ops), have no embedded backend
//! (`candle`), or give up ESP-NN's ~7× SIMD speedup the
//! ESP32-S3 PIE / DSP extensions deliver (custom interpreter).
//! `esp-tflite-micro` is what ESPHome ships in production and
//! lands microWakeWord inference well under our 200 ms latency
//! budget.
//!
//! ## Build host
//!
//! Only builds on the `xtensa-esp32s3-none-elf` target via the
//! `esp` Rust toolchain. Host builds intentionally fail — the
//! crate has nothing useful to expose on x86_64 and pretending
//! otherwise would just push the failure to wake-word task spawn
//! time.

#![no_std]
// `-sys` crates by definition cross the safe/unsafe boundary —
// every `extern "C"` declaration and every call into it through
// the public surface is `unsafe`. The workspace-wide
// `unsafe_code = "deny"` is the right default for Rust crates;
// here we narrow it to "allowed but each block needs a
// SAFETY comment" via the per-file allow + the clippy lint that
// flags missing safety docs.
#![allow(unsafe_code)]
#![warn(clippy::missing_safety_doc, clippy::undocumented_unsafe_blocks)]
// Spec-heavy reference module: TFLM, ESP-NN, MicroInterpreter,
// microWakeWord, MixConv, ESP-DSP / PIE, etc. appear throughout
// the docs. Per-occurrence backticking would bury the prose.
// Same pattern as `stackchan-net::blufi`'s module-level allow.
#![allow(clippy::doc_markdown)]

use core::marker::PhantomData;
use core::ptr::NonNull;

extern crate alloc;

/// C++ runtime bridge — `runtime_stubs.cpp` calls these in lieu of
/// linking libstdc++. Routing through the Rust global allocator
/// keeps both sides on the same `esp-alloc` PSRAM heap so heap
/// accounting is unified across the FFI boundary. A small
/// `usize`-sized header prefix records the user-visible size so
/// `free` can recover the original `Layout`.
mod runtime {
    use core::alloc::Layout;

    use alloc::alloc::{alloc as rust_alloc, dealloc as rust_dealloc};

    /// 16-byte alignment is `max_align_t` on xtensa-esp32s3 (covers
    /// `double` + pointer pair) — the strictest alignment any
    /// C++-side `operator new` call could need.
    const ALIGN: usize = 16;
    /// Width of the size-header slot prepended to every allocation,
    /// padded to `ALIGN` so the user-visible pointer returned by
    /// `etms_runtime_alloc` (`raw + HEADER_SIZE`) inherits the
    /// raw allocation's 16-byte alignment. A bare
    /// `size_of::<usize>()` (4 bytes on this target) would land
    /// 8- and 16-byte C++ allocations on a 4-byte boundary and
    /// trip `LoadStoreAlignmentCause` on `double` / SIMD loads.
    const HEADER_SIZE: usize = ALIGN;

    /// SAFETY: the only caller is C++ `operator new` in
    /// `runtime_stubs.cpp`. Returns null on alloc failure to match
    /// the `nothrow` contract the C++ side relies on.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn etms_runtime_alloc(size: usize) -> *mut u8 {
        let Some(total) = size.checked_add(HEADER_SIZE) else {
            return core::ptr::null_mut();
        };
        let Ok(layout) = Layout::from_size_align(total, ALIGN) else {
            return core::ptr::null_mut();
        };
        // SAFETY: layout is well-formed.
        let raw = unsafe { rust_alloc(layout) };
        if raw.is_null() {
            return core::ptr::null_mut();
        }
        // SAFETY: `raw` has at least `HEADER_SIZE` bytes per the
        // layout. `write_unaligned` sidesteps clippy's alignment
        // lint without changing semantics — `raw` is always
        // 16-byte aligned per `ALIGN`, so any usize write is
        // naturally aligned in practice.
        unsafe { core::ptr::write_unaligned(raw.cast::<usize>(), size) };
        // SAFETY: `raw + HEADER_SIZE` is inside the allocation.
        unsafe { raw.add(HEADER_SIZE) }
    }

    /// C++ `abort()` landing pad — TFLM's `TFLITE_DCHECK_*` macros
    /// route here on assertion failure. Surface a `panic!` so the
    /// firmware's panic handler emits a defmt trace over the
    /// USB-Serial-JTAG before halting; without this the C++ side
    /// spin-loops with no diagnostic and any TFLM programmer error
    /// looks indistinguishable from a hang.
    #[unsafe(no_mangle)]
    #[allow(
        clippy::panic,
        reason = "deliberate panic — routes TFLM's abort() through the firmware's defmt panic handler"
    )]
    pub extern "C" fn etms_runtime_abort() -> ! {
        panic!("esp-tflite-micro-sys: C++ abort() reached (likely TFLITE_DCHECK)");
    }

    /// SAFETY: caller is C++ `operator delete` and only passes
    /// pointers previously returned by `etms_runtime_alloc`. Null
    /// is tolerated (matches `delete nullptr`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn etms_runtime_free(ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ptr` came from `etms_runtime_alloc`, so backing
        // up by `HEADER_SIZE` lands on the size header.
        let header = unsafe { ptr.sub(HEADER_SIZE) };
        // SAFETY: header slot was written during alloc. Same
        // alignment story as the matching `write_unaligned`.
        let size = unsafe { core::ptr::read_unaligned(header.cast::<usize>()) };
        let Ok(layout) = Layout::from_size_align(size + HEADER_SIZE, ALIGN) else {
            return;
        };
        // SAFETY: `header` + `layout` matches the original allocation.
        unsafe { rust_dealloc(header, layout) };
    }
}

/// bindgen-generated `extern "C"` declarations for the C-ABI shim.
mod ffi {
    // The shim header is pure C and uses only `<stdint.h>` and
    // `<stddef.h>`, so bindings carry no `std::` dependencies and
    // embed cleanly into `no_std`. Documentation lints are
    // disabled here because the generated declarations are
    // mechanical — the contract lives in the C header and the
    // safe wrappers above.
    #![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
    #![allow(dead_code, missing_docs, clippy::missing_docs_in_private_items)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// One-time TFLM port-layer initialisation. Idempotent.
///
/// The default `tflite::InitializeTarget()` is `{}` on this port;
/// the call exists so the linker has a reason to keep the
/// port-surface symbols around.
pub fn init() {
    // SAFETY: `etms_init` is a no-op wrapper around the empty
    // `tflite::InitializeTarget()` body; no preconditions, no
    // mutable global state.
    unsafe { ffi::etms_init() };
}

/// `TfLiteStatus` code surfaced by failed inference operations.
///
/// A thin newtype around the C enum value so a future module can
/// map specific codes (`kTfLiteError`, `kTfLiteDelegateError`, …)
/// to typed Rust errors. `0` means success and is unwrapped at the
/// `Result` boundary; only non-zero values reach this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TfLiteStatus(
    /// Raw `TfLiteStatus` value as returned by the underlying
    /// C++ call. Non-zero by construction.
    pub core::ffi::c_int,
);

/// Operator resolver — table of op kernels the interpreter consults
/// when executing a model.
///
/// Construct one, register the op kernels the model needs via
/// `add_*` accessors, then pass the resolver into [`Interpreter::new`].
/// The resolver is empty on construction; an interpreter built over
/// an empty resolver will fail [`Interpreter::allocate_tensors`] for
/// any model that uses real ops.
pub struct Resolver {
    /// Owned `MicroMutableOpResolver<20>` instance allocated on
    /// the C++ heap by the shim.
    handle: NonNull<ffi::EtmsResolver>,
}

impl Resolver {
    /// Allocates an empty resolver. Returns `None` if the
    /// underlying heap allocation fails.
    #[must_use]
    pub fn new() -> Option<Self> {
        // SAFETY: `etms_resolver_create` has no parameters and no
        // preconditions. On failure it returns null, which we map
        // to `None` via `NonNull::new`.
        let h = unsafe { ffi::etms_resolver_create() };
        NonNull::new(h).map(|handle| Self { handle })
    }
}

/// Shared body for every `Resolver::add_*` method. Each invocation
/// expands to a thin `Result`-returning wrapper around the
/// corresponding `etms_resolver_add_*` FFI call. The macro keeps the
/// 20-op surface declarative — the resolver's capacity is 20, the
/// op set is fixed by microWakeWord, and there's no per-op logic
/// beyond status passthrough.
macro_rules! resolver_add_methods {
    ($($(#[$attr:meta])* $rust_name:ident => $ffi_name:ident),* $(,)?) => {
        impl Resolver {
            $(
                $(#[$attr])*
                ///
                /// # Errors
                ///
                /// Returns the resolver's `TfLiteStatus` — non-zero
                /// when the resolver's capacity (20 ops) is exceeded
                /// or when the same op is registered twice.
                pub fn $rust_name(&mut self) -> Result<(), TfLiteStatus> {
                    // SAFETY: `self.handle` is a valid resolver
                    // pointer (held in `NonNull`); the FFI call only
                    // dereferences it for the duration of the call.
                    let status = unsafe { ffi::$ffi_name(self.handle.as_ptr()) };
                    if status == 0 {
                        Ok(())
                    } else {
                        Err(TfLiteStatus(status))
                    }
                }
            )*
        }
    };
}

resolver_add_methods! {
    /// Registers `BuiltinOperator_ADD` (elementwise add, ESP-NN-accelerated).
    add_add => etms_resolver_add_add,
    /// Registers `BuiltinOperator_ASSIGN_VARIABLE` (resource-variable write).
    add_assign_variable => etms_resolver_add_assign_variable,
    /// Registers `BuiltinOperator_AVERAGE_POOL_2D` (ESP-NN-accelerated).
    add_average_pool_2d => etms_resolver_add_average_pool_2d,
    /// Registers `BuiltinOperator_CALL_ONCE` (fires at first invocation).
    add_call_once => etms_resolver_add_call_once,
    /// Registers `BuiltinOperator_CONCATENATION`.
    add_concatenation => etms_resolver_add_concatenation,
    /// Registers `BuiltinOperator_CONV_2D` (ESP-NN-accelerated).
    add_conv_2d => etms_resolver_add_conv_2d,
    /// Registers `BuiltinOperator_DEPTHWISE_CONV_2D` (ESP-NN-accelerated).
    add_depthwise_conv_2d => etms_resolver_add_depthwise_conv_2d,
    /// Registers `BuiltinOperator_FULLY_CONNECTED` (ESP-NN-accelerated).
    add_fully_connected => etms_resolver_add_fully_connected,
    /// Registers `BuiltinOperator_LOGISTIC` (sigmoid; reference kernel).
    add_logistic => etms_resolver_add_logistic,
    /// Registers `BuiltinOperator_MAX_POOL_2D` (ESP-NN-accelerated).
    add_max_pool_2d => etms_resolver_add_max_pool_2d,
    /// Registers `BuiltinOperator_MEAN` (reference reduction kernel).
    add_mean => etms_resolver_add_mean,
    /// Registers `BuiltinOperator_MUL` (elementwise multiply, ESP-NN-accelerated).
    add_mul => etms_resolver_add_mul,
    /// Registers `BuiltinOperator_PACK`.
    add_pack => etms_resolver_add_pack,
    /// Registers `BuiltinOperator_PAD` (zero-pad along tensor dims).
    add_pad => etms_resolver_add_pad,
    /// Registers `BuiltinOperator_QUANTIZE`.
    add_quantize => etms_resolver_add_quantize,
    /// Registers `BuiltinOperator_READ_VARIABLE` (resource-variable read).
    add_read_variable => etms_resolver_add_read_variable,
    /// Registers `BuiltinOperator_RESHAPE`.
    add_reshape => etms_resolver_add_reshape,
    /// Registers `BuiltinOperator_SPLIT_V` (variable-size split).
    add_split_v => etms_resolver_add_split_v,
    /// Registers `BuiltinOperator_STRIDED_SLICE`.
    add_strided_slice => etms_resolver_add_strided_slice,
    /// Registers `BuiltinOperator_VAR_HANDLE` (resource-variable allocation).
    add_var_handle => etms_resolver_add_var_handle,
}

impl Drop for Resolver {
    fn drop(&mut self) {
        // SAFETY: `self.handle` was obtained from a successful
        // `etms_resolver_create` call and hasn't been freed yet
        // (Drop runs at most once per instance). `etms_resolver_destroy`
        // tolerates non-null pointers exclusively, which `NonNull`
        // guarantees.
        unsafe { ffi::etms_resolver_destroy(self.handle.as_ptr()) };
    }
}

/// Constructed interpreter, ready for [`Interpreter::allocate_tensors`]
/// and [`Interpreter::invoke`].
///
/// The handle borrows from the model bytes, the resolver, and the
/// tensor arena passed to [`Interpreter::new`] — all three must
/// outlive the interpreter, expressed here by a shared `'a`
/// lifetime parameter.
pub struct Interpreter<'a> {
    /// Owned `MicroInterpreter` instance allocated on the C++ heap
    /// by the shim. Borrows the model, resolver, and arena passed
    /// at construction.
    handle: NonNull<ffi::EtmsInterpreter>,
    /// Ties the lifetime of the borrowed model/resolver/arena to
    /// the interpreter handle so the borrow checker rejects
    /// dropping them while the interpreter is alive.
    _phantom: PhantomData<&'a mut ()>,
}

impl<'a> Interpreter<'a> {
    /// Constructs an interpreter over `model`, using `resolver`'s
    /// registered op kernels and `arena` for tensor storage.
    ///
    /// Returns `None` if the model bytes are malformed (bad magic,
    /// wrong schema version) or if the underlying heap allocation
    /// fails. The arena is *not* sized-checked here — that happens
    /// at [`Interpreter::allocate_tensors`] time, when TFLM's memory
    /// planner determines how much space the model actually needs.
    #[must_use]
    pub fn new(model: &'a [u8], resolver: &'a Resolver, arena: &'a mut [u8]) -> Option<Self> {
        // SAFETY: all three pointer arguments are derived from
        // Rust references with at-least-`'a` lifetime, so they
        // remain valid (and the arena exclusively borrowed) for
        // the returned handle's lifetime. The shim validates the
        // model bytes via `tflite::GetModel` + schema-version
        // check and returns null on rejection.
        let h = unsafe {
            ffi::etms_interpreter_create(
                model.as_ptr(),
                model.len(),
                resolver.handle.as_ptr(),
                arena.as_mut_ptr(),
                arena.len(),
            )
        };
        NonNull::new(h).map(|handle| Self {
            handle,
            _phantom: PhantomData,
        })
    }

    /// Runs the memory planner and allocates tensor storage inside
    /// the arena. Must be called before [`Interpreter::invoke`].
    ///
    /// # Errors
    ///
    /// Returns the underlying `TfLiteStatus` code on failure. Common
    /// causes are: the arena being too small for the model's
    /// tensors, the model referencing an op the resolver doesn't
    /// know about, or planner constraint violations. The detailed
    /// diagnostic is logged via `DebugLog`, which is currently
    /// silent under `TF_LITE_STRIP_ERROR_STRINGS`.
    pub fn allocate_tensors(&mut self) -> Result<(), TfLiteStatus> {
        // SAFETY: `self.handle` is a valid, exclusively-borrowed
        // interpreter pointer for the duration of `&mut self`.
        let status = unsafe { ffi::etms_interpreter_allocate_tensors(self.handle.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err(TfLiteStatus(status))
        }
    }

    /// Runs one inference pass. Caller must have populated the
    /// input tensors since the last invocation; output tensors
    /// hold the result on success.
    ///
    /// # Errors
    ///
    /// Returns the underlying `TfLiteStatus` code if the planner
    /// hasn't run ([`Interpreter::allocate_tensors`] was not called
    /// or failed), if an op kernel returns an error, or if the
    /// model graph is internally inconsistent.
    pub fn invoke(&mut self) -> Result<(), TfLiteStatus> {
        // SAFETY: same reasoning as `allocate_tensors`.
        let status = unsafe { ffi::etms_interpreter_invoke(self.handle.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err(TfLiteStatus(status))
        }
    }

    /// Number of input tensors the loaded model declares.
    ///
    /// Stable across the interpreter's lifetime; valid even before
    /// [`Interpreter::allocate_tensors`] (TFLM reads the count from
    /// the flatbuffer header at construction time).
    #[must_use]
    pub fn inputs_len(&self) -> usize {
        // SAFETY: `self.handle` is a valid interpreter pointer for
        // the duration of `&self`. The accessor reads from the
        // model's graph metadata and doesn't mutate.
        unsafe { ffi::etms_interpreter_inputs_size(self.handle.as_ptr()) }
    }

    /// Number of output tensors the loaded model declares.
    #[must_use]
    pub fn outputs_len(&self) -> usize {
        // SAFETY: as in `inputs_len`.
        unsafe { ffi::etms_interpreter_outputs_size(self.handle.as_ptr()) }
    }

    /// Writable byte view of the `idx`-th input tensor.
    ///
    /// Returns `None` if `idx >= self.inputs_len()` or if the
    /// underlying tensor doesn't have a backing buffer yet (call
    /// [`Interpreter::allocate_tensors`] first). The returned
    /// slice's contents persist across [`Interpreter::invoke`]
    /// calls — TFLM reads inputs from the buffer the caller writes
    /// here and writes outputs to a separate buffer obtained via
    /// [`Interpreter::output_bytes`].
    ///
    /// The borrow shape (`&mut self`) is what's required to prevent
    /// `invoke` from running while a caller holds a writable
    /// reference to the input arena; this is sound but conservative —
    /// reading other tensors through an immutable accessor while
    /// holding an input borrow is also rejected, even though that
    /// would be safe.
    #[must_use]
    pub fn input_bytes_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        // SAFETY: the FFI returns either null (out-of-range or no
        // arena) or a pointer into the tensor arena. The arena
        // lives for `'a` (longer than `self`), and `&mut self`
        // guarantees no other reference exists into the interpreter
        // for the slice's lifetime.
        unsafe {
            let data = ffi::etms_interpreter_input_data(self.handle.as_ptr(), idx);
            let bytes = ffi::etms_interpreter_input_bytes(self.handle.as_ptr(), idx);
            if data.is_null() || bytes == 0 {
                None
            } else {
                Some(core::slice::from_raw_parts_mut(data, bytes))
            }
        }
    }

    /// Readable byte view of the `idx`-th output tensor.
    ///
    /// Returns `None` if `idx >= self.outputs_len()` or if the
    /// underlying tensor doesn't have a backing buffer yet (call
    /// [`Interpreter::allocate_tensors`] first). The returned
    /// slice's contents reflect the result of the last
    /// [`Interpreter::invoke`] call — calling this *before* the
    /// first invocation returns a buffer of zero (or
    /// implementation-defined) bytes.
    #[must_use]
    pub fn output_bytes(&self, idx: usize) -> Option<&[u8]> {
        // SAFETY: the FFI returns either null (out-of-range or no
        // arena) or a pointer into the tensor arena. The arena
        // lives for `'a` (longer than `self`); `&self` is enough
        // for read-only access because TFLM doesn't mutate output
        // tensors except during `invoke`, which takes `&mut self`.
        unsafe {
            let data = ffi::etms_interpreter_output_data(self.handle.as_ptr(), idx);
            let bytes = ffi::etms_interpreter_output_bytes(self.handle.as_ptr(), idx);
            if data.is_null() || bytes == 0 {
                None
            } else {
                Some(core::slice::from_raw_parts(data, bytes))
            }
        }
    }
}

impl Drop for Interpreter<'_> {
    fn drop(&mut self) {
        // SAFETY: handle is non-null and was obtained from a
        // successful `etms_interpreter_create` call. The borrows
        // we hold (`'a`) outlive this Drop because Rust's
        // drop order processes `Interpreter` before the
        // references it borrows.
        unsafe { ffi::etms_interpreter_destroy(self.handle.as_ptr()) };
    }
}
