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
