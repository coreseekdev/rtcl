//! rtcl-jit — compile rtcl ByteCode to WebAssembly modules.
//!
//! JIT is ByteCode's second consumer (rtcl-core's `vm_exec` is the first):
//! it takes a [`rtcl_ir::ByteCode`] produced by `rtcl-parser`'s compiler and
//! emits a small self-contained wasm module (HANDOFF §5).  Architecture:
//!
//! - **Granularity = per-proc module.**  One module per compilation unit;
//!   invalidation discards a single instance (Mono jiterpreter model).
//! - **Cross-boundary values = u32 handles** into a runtime-side value
//!   arena owned by the host; no host linear-memory imports.
//! - **Errors use Tcl result codes**, not wasm exceptions: the exported
//!   `run()` returns `i32` (0 = OK, 1 = error, 3 = break, 4 = continue,
//!   mirroring tclsh's TCL_OK/TCL_ERROR/TCL_BREAK/TCL_CONTINUE) and the
//!   result value is delivered through the arena (`rtcl.set_result`).
//! - **Host ABI** (import module `rtcl`, grown milestone by milestone):
//!   - `push_int(v: i64) -> u32` — push a canonical i64, get its handle
//!   - `push_empty() -> u32` — push the empty value
//!   - `set_result(h: u32)` — record the program's result value
//! - **Fallback**: anything the emitter cannot compile byte-exactly is
//!   rejected ([`emit::Unsupported`]) and the caller keeps the interpreter
//!   path.  The epoch guard (registry tier1_epoch) is wired at M3.
//!
//! Platform: the emitter is pure Rust and compiles everywhere.  The
//! instantiation engines are feature-gated — `jit-wasm` (js-sys, the
//! primary wasm32-unknown-unknown path) and `jit-native` (wasmtime; lands
//! with M1).  wasip1 / embedded no_std never JIT.
#![forbid(unsafe_code)]

pub mod emit;

pub mod engine;

/// Shorthand for the emitter's rejection type.
pub use emit::Unsupported;
