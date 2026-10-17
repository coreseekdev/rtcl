//! ABI v2 — the host/JIT module contract (设计文档 §3).
//!
//! ```wat
//! (global (import "rtcl" "fuel")    (mut i64))   ;; 步数预算: 回边自减, 宿主充值
//! (global (import "rtcl" "epoch")   (mut i64))   ;; 失效守卫: 入口+回边比对
//! (global (import "rtcl" "caching") (mut i32))   ;; 变量缓存许可: 宿主首次
//!                                                 ;; rtcl.call 后清零 (M1' 策略)
//! (import "rtcl" "unbox_int"   (func (param i32) (result i32 i64)))
//! (import "rtcl" "unbox_float" (func (param i32) (result i32 f64)))
//! (import "rtcl" "box"         (func (param i64)  (result i32)))
//! (import "rtcl" "box_float"   (func (param f64)  (result i32)))
//! (import "rtcl" "binary_op"   (func (param i32 i64 i64) (result i64)))
//! (import "rtcl" "get_var"     (func (param i32)  (result i64)))
//! (import "rtcl" "set_var"     (func (param i32 i64)))
//! (import "rtcl" "call"        (func (param i32 i32 i32) (result i64)))
//! (import "rtcl" "set_result"  (func (param i64)))
//! ```
//!
//! M1' 在 §3 基础上的**加法性**扩展（冷路径/驱动协作，§3 签名原样保留）:
//!
//! ```wat
//! (import "rtcl" "refuel"    (func (result i64)))          ;; fuel 用尽: 宿主充值并
//!                                                            ;; 返回剩余量; 2 = 预算耗尽
//! (import "rtcl" "fail"      (func (param i32)))           ;; 冷路径: 报告失败点
//!                                                            ;; (静态 fail-info 表索引)
//! (import "rtcl" "bool_test" (func (param i64) (result i64))) ;; strict_bool 兜底
//! (import "rtcl" "incr_var"  (func (param i32 i64) (result i64))) ;; incr 全语义
//! (import "rtcl" "push_const" (func (param i32) (result i64)))  ;; 常量池句柄化
//! (import "rtcl" "exit"      (func (param i32)))            ;; Exit op → 宿主终局
//! ```
//!
//! 值表示 = tagged i64（与宿主 `Value` 8 字节形状同构, 设计文档 §2）:
//! bit0=1 → i63 立即数 (`n<<1|1`); bit0=0 → 宿主 arena 句柄; **bits==2 = 错误哨兵**
//! （宿主已置 pending error, JIT 见之即 `br` 到错误出口返回 TCL_ERROR）。
//!
//! `run() -> i32` 返回 Tcl result code; `STALE = 5` 为 epoch 失效专用。

/// Tcl result codes returned by the exported `run()`.
pub mod code {
    pub const OK: i32 = 0;
    pub const ERROR: i32 = 1;
    pub const RETURN: i32 = 2;
    pub const BREAK: i32 = 3;
    pub const CONTINUE: i32 = 4;
    /// Epoch mismatch — the module is stale; the caller must drop it and
    /// re-dispatch on the interpreter (设计文档 §3).
    pub const STALE: i32 = 5;
    /// Exit op（加法扩展）：驱动复刻 rtcl 文件模式的 `exit with value` 行为。
    pub const EXIT: i32 = 6;
}

/// arena 句柄编码：句柄字（tagged word 形态，bit0=0）= `arena 下标 << 1`。
/// arena[0]=UNSET（永不产出）、arena[1]=哨兵保留（句柄 2 = 错误哨兵，
/// 永不作为值出现）、arena[2]=空值（句柄 4）、arena[3..]=常量池。
#[inline]
pub const fn handle(index: u32) -> i64 {
    ((index as u64) << 1) as i64
}

/// 常量池第 `idx` 个字符串的句柄。
#[inline]
pub const fn const_handle(idx: u32) -> i64 {
    handle(CONST_ARENA_BASE + idx)
}

/// 常量池在 arena 中的起始下标。
pub const CONST_ARENA_BASE: u32 = 3;

/// 空值的句柄（arena[2]）。
pub const EMPTY_HANDLE: i64 = 4;

/// The error sentinel as a raw tagged word (bits == 2).
pub const ERR_SENTINEL: i64 = 2;

/// Tag an i63 immediate into its wasm representation.
#[inline]
pub const fn tag_imm(n: i64) -> i64 {
    ((n as u64) << 1) as i64 | 1
}

/// Untag a tagged immediate (arithmetic shift; tag bit known to be 1).
#[inline]
pub const fn untag(v: i64) -> i64 {
    v >> 1
}

/// Is this raw word a tagged immediate (bit0 == 1)?
#[inline]
pub const fn is_imm(v: i64) -> bool {
    v & 1 == 1
}

/// `binary_op` opcode space (the `op` first parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BinOp {
    Add = 0,
    Sub = 1,
    Mul = 2,
    Div = 3,
    Mod = 4,
    Pow = 5,
    Neg = 6,
    Not = 7,
    BitAnd = 8,
    BitOr = 9,
    BitXor = 10,
    BitNot = 11,
    Shl = 12,
    Shr = 13,
    Eq = 14,
    Ne = 15,
    Lt = 16,
    Gt = 17,
    Le = 18,
    Ge = 19,
    StrEq = 20,
    StrNe = 21,
    And = 22,
    Or = 23,
}

/// Import index of each ABI function inside the emitted module (order is
/// the import-section order `assemble` emits — single source of truth).
pub mod import {
    pub const UNBOX_INT: u32 = 0;
    pub const UNBOX_FLOAT: u32 = 1;
    pub const BOX: u32 = 2;
    pub const BOX_FLOAT: u32 = 3;
    pub const BINARY_OP: u32 = 4;
    pub const GET_VAR: u32 = 5;
    pub const SET_VAR: u32 = 6;
    pub const CALL: u32 = 7;
    pub const SET_RESULT: u32 = 8;
    pub const REFUEL: u32 = 9;
    pub const FAIL: u32 = 10;
    pub const BOOL_TEST: u32 = 11;
    pub const INCR_VAR: u32 = 12;
    pub const PUSH_CONST: u32 = 13;
    pub const EXIT: u32 = 14;
    /// The one locally defined function (`run`) comes after the imports.
    pub const RUN: u32 = 15;
    /// Number of imported functions (globals come first in the import
    /// section but do not shift function indices).
    pub const COUNT: u32 = 15;
}

/// Fuel policy: each loop back-edge decrements the imported `fuel`
/// global; at zero the module calls `rtcl.refuel()` and the host recharges
/// in chunks of [`REFUEL_CHUNK`] steps (each chunk charges the
/// interpreter's step budget once — see exec.rs for the exactness note).
pub const REFUEL_CHUNK: i64 = 1024;

/// Var-cache memory layout (one page; offsets in bytes).
pub mod cache {
    /// Cached tagged values, `MAX_VARS` × i64.
    pub const VALUES: u32 = 0;
    /// Per-var state byte: 0 = absent (zero-init), 1 = clean, 2 = dirty.
    pub const STATE: u32 = 512;
    pub const MAX_VARS: usize = 64;
}

/// wasm-encoder type-section indices for the function signatures.
pub mod ty {
    /// `() -> i32` — `run`.
    pub const RUN: u32 = 0;
    /// `(i64) -> i32` — box.
    pub const BOX: u32 = 1;
    /// `(f64) -> i32` — box_float.
    pub const BOX_FLOAT: u32 = 2;
    /// `(i32, i64, i64) -> i64` — binary_op.
    pub const BINARY_OP: u32 = 3;
    /// `(i32) -> i64` — get_var / bool_test / push_const.
    pub const I32_TO_I64: u32 = 4;
    /// `(i32, i64) -> i32` — set_var（返回 0=ok/1=错误；ABI 加法性扩展：
    /// §3 草图无返回值，存储错误信令需要状态通道）。
    pub const SET_VAR: u32 = 5;
    /// `(i32, i32, i32) -> i64` — call.
    pub const CALL: u32 = 6;
    /// `(i64) -> ()` — set_result.
    pub const SET_RESULT: u32 = 7;
    /// `() -> i64` — refuel.
    pub const REFUEL: u32 = 8;
    /// `(i32) -> i32` — fail（返回 0=纯错误/1..4=控制流穿越 kind）。
    pub const FAIL: u32 = 9;
    /// `(i32) -> (i32, i64)` — unbox_int (multi-value, Wasm 2.0).
    pub const UNBOX_INT: u32 = 10;
    /// `(i32) -> (i32, f64)` — unbox_float (multi-value).
    pub const UNBOX_FLOAT: u32 = 11;
    /// `(i32, i64) -> i64` — incr_var（同 SET_VAR 形状、结果 tagged）。
    pub const INCR_VAR: u32 = 12;
    /// `(i32) -> ()` — exit。
    pub const I32_VOID: u32 = 13;
    /// 兼容别名（assemble 用）。
    pub const I32_VOID2: u32 = 13;
    /// `(i64) -> i64` — bool_test（输入 tagged word，输出 tagged 0/1）。
    pub const BOOL_TEST: u32 = 14;
}
