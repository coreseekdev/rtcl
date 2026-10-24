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
    pub const CONCAT: u32 = 15;
    pub const FOREACH_START: u32 = 16;
    pub const FOREACH_BIND: u32 = 17;
    pub const FOREACH_COLLECT: u32 = 18;
    pub const FOREACH_END: u32 = 19;
    pub const EVAL_SCRIPT: u32 = 20;
    pub const EVAL_EXPR: u32 = 21;
    pub const LAPPEND_VAR: u32 = 22;
    /// `(i32, i32, i32) -> i64` — call_argv(base, argc, site)：直连分派
    /// （M2 全案：已求值实参走 dispatch_values，免站点原文 eval 全链）。
    pub const CALL_ARGV: u32 = 23;
    /// `(i32, i32, i32) -> i64` — resolve_proc(site, argc, argv_base)：
    /// fast-tier 调用点冷融合——解析目标 proc→槽号回填 site-cache 并
    /// 代打一次；不可 fast 化 → cache=-1（后续走 call_argv 回落）。
    pub const RESOLVE_PROC: u32 = 24;
    /// `(i32) -> ()` — proc_exit_frame(slot)：fast 错误冷路径，本层补
    /// `(procedure "name" line N)` 帧。
    pub const PROC_EXIT_FRAME: u32 = 25;
    /// The one locally defined function (`run`) comes after the imports.
    pub const RUN: u32 = 26;
    /// fast-tier 的 fast_impl（run 之后、proc_main 之前）。
    pub const FAST_IMPL: u32 = 27;
    /// Number of imported functions (globals come first in the import
    /// section but do not shift function indices).
    pub const COUNT: u32 = 26;
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
    /// 可变参数 op（Concat/foreach/call_argv）的暂存区：n 个 tagged
    /// word，8 字节/个。上限 = CallSite 的 argc ≤ 16 + 常规可变 op。
    pub const SCRATCH: u32 = 1024;
    /// fast-tier 参数区（per-depth 切片：base = PARAMS + depth*PARAM_STRIDE，
    /// 由 proc_main wrapper 拷贝实参——fast_impl 的 LoadVar(参数) 从
    /// 此读，递归各层互不覆盖）。
    pub const PARAMS: u32 = 8192;
    /// 每深度参数区（8B × 最大参数数 16）。
    pub const PARAM_STRIDE: u32 = 128;
    /// fast-tier site-cache：每 proc 一段（SITE_STRIDE 字节），槽号
    /// i32×site——由宿主在实例化时设 per-instance global
    /// site_cache_base。SCRATCH 之后留 1KB 余量。
    pub const SITE_CACHE_BASE: u32 = 4096;
    /// 每 proc 的 site-cache 段长（i32 × 1024 sites）。
    pub const SITE_STRIDE: u32 = 4096;
}

/// wasm-encoder type-section indices for the function signatures
/// （与 assemble 的发射序一致——单一事实源在 assemble，修改须同步）。
pub mod ty {
    pub const RUN: u32 = 0;
    pub const BOX: u32 = 1;
    pub const BOX_FLOAT: u32 = 2;
    pub const BINARY_OP: u32 = 3;
    pub const I32_TO_I64: u32 = 4;
    pub const SET_VAR: u32 = 5;
    pub const CALL: u32 = 6;
    pub const SET_RESULT: u32 = 7;
    pub const REFUEL: u32 = 8;
    pub const FAIL: u32 = 9;
    pub const UNBOX_INT: u32 = 10;
    pub const UNBOX_FLOAT: u32 = 11;
    pub const INCR_VAR: u32 = 12;
    pub const I32_VOID: u32 = 13;
    pub const BOOL_TEST: u32 = 14;
    pub const CONCAT_TY: u32 = 15;
    pub const FOREACH_START: u32 = 16;
    pub const FOREACH_BIND: u32 = 17;
    pub const FOREACH_COLLECT: u32 = 18;
    pub const FOREACH_END_TY: u32 = 19;
    pub const I64_TO_I64: u32 = 20;
    /// fast-tier proc_main：`(i32 slot, i32 argc, i32 argv) -> (i64 val, i32 rc)`
    /// ——多值返回贯穿 wasm 层（控制流经 rc 传播免宿主）。
    /// 注意：RUN..CONCAT 共 22 项（0-21）——fast 两型在表尾。
    pub const PROC_MAIN: u32 = 22;
    /// fast-tier fast_impl（无参，局部布局与 run 完全一致——proc_main
    /// 参数占 local 0-2 会推移值栈寄存器，故薄 wrapper + 独立体函数）。
    pub const PROC_IMPL: u32 = 23;
}
