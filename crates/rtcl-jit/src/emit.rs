//! ByteCode → wasm 模块发射门面（nanopass 管线的入口，§7.3）。
//!
//! M0 的单函数发射器已按 ABI v2 重构为 pass 链（`crate::pass`）；本模块
//! 保留稳定入口：`compile_module` 返回 wasm 字节 + 中间层产物
//! （L2/L3 供 golden 测试与驱动侧 fail 表消费）。

use crate::ir::{L2, L3};
use crate::pass::{self, EmitConfig};
use rtcl_ir::bytecode::ByteCode;

/// The emitter met an op it cannot compile byte-exactly.  The payload is
/// the op's display form (as printed by `disassemble`) for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "jit: unsupported: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

/// 编译产物：wasm 字节 + 编译期决策（驱动需要 `l2.fails` 复刻错误协议、
/// `l2.vars`/`const_base` 预驻留 arena）。
pub struct Module {
    pub bytes: Vec<u8>,
    pub l2: L2,
    pub l3: L3,
    /// fast-tier 元数据（assemble 的 fast gate 命中时）：
    /// (nparams, my_epoch)——proc_main 导出已发射，驱动可入共享
    /// Store + funcref 表（call_indirect 直呼协议）。
    pub fast: Option<FastMeta>,
}

/// fast-tier 元数据。
#[derive(Debug, Clone, Copy)]
pub struct FastMeta {
    /// proc 参数个数（params_aligned 语义下的精确个数）。
    pub nparams: usize,
    /// 编译期 tier1 epoch（CallSite 槽缓存的失效基准）。
    pub epoch: u64,
}

/// Compile a [`ByteCode`] to a wasm module via the nanopass pipeline.
///
/// Returns [`Unsupported`] when the unit is outside the accepted subset —
/// callers must keep the interpreter path in that case; nothing partial is
/// ever emitted（「编译拒绝」粒度，设计文档 §2）。
pub fn compile_module(code: &ByteCode) -> Result<Vec<u8>, Unsupported> {
    compile(code, &EmitConfig::default()).map(|m| m.bytes)
}

/// 全产物编译（含 L2/L3）。
pub fn compile(code: &ByteCode, cfg: &EmitConfig) -> Result<Module, Unsupported> {
    let (bytes, l2, l3, fast) = pass::run(code, cfg)?;
    Ok(Module { bytes, l2, l3, fast })
}
