//! rtcl-jit — compile rtcl ByteCode to WebAssembly modules（ABI v2，
//! 设计文档 `docs/JIT-WASM-GC.md` §3）。
//!
//! JIT is ByteCode's second consumer (rtcl-core's `vm_exec` is the first):
//! it takes a [`rtcl_ir::ByteCode`] produced by `rtcl-parser`'s compiler and
//! emits a small self-contained wasm module per compilation unit
//! (Mono jiterpreter model).  M1' architecture:
//!
//! - **值表示 = tagged i64**（与宿主 `Value` 8 字节形状同构，§2 三原则）:
//!   bit0=1 → i63 立即数；bit0=0 → 宿主 arena 句柄；bits==2 → 错误哨兵。
//! - **ABI v2**（§3 全文见 [`abi`]）：宿主 import `rtcl`（函数 +
//!   fuel/epoch/caching 可变 global）；`run() -> i32` 返回 Tcl result
//!   code（STALE=5 = epoch 失效）。
//! - **Int 快车道**（§4）：比较免 untag、add/sub 内联+范围检查、mul 高
//!   32 位检查、div/mod 全走宿主、位运算内联；守卫失败调 `binary_op`
//!   宿主兜底（shimmer/bignum/报错全在宿主）。
//! - **fuel/epoch imported mutable globals**（§3）：循环回边 fuel 自减、
//!   入口+回边 epoch 比对——`while {1} {}` 类死循环受步数预算约束。
//! - **nanopass 管线**（§7.3）：`pass::` 下 L0→L1→L2→L3→L4 的单职责
//!   pass 链，每 pass 可 golden 测试，`check::` 为逐层校验器。
//! - **回退两粒度**（§2）：编译拒绝（整单元留解释器）vs 守卫回退
//!   （运行时调宿主 full-semantics import 继续）。
//!
//! Platform: the emitter is pure Rust and compiles everywhere.  Engines are
//! feature-gated — `jit-wasm` (js-sys), `jit-native` (wasmtime), `jit-exec`
//! (wasmi in-process executor + `jitsh` binary).  wasip1 / embedded no_std
//! never JIT.

#![forbid(unsafe_code)]

pub mod abi;
pub mod check;
pub mod ir;
pub mod pass;

pub mod emit;
pub mod engine;

#[cfg(feature = "jit-exec")]
pub mod exec;

/// Shorthand for the emitter's rejection type.
pub use emit::Unsupported;
