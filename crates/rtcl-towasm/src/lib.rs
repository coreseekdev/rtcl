//! towasm —— Tcl → wasm 产物编译库（独立线）。
//!
//! 产品 = .wasm 字节 + rtcl.* 运行时契约：任意 WASM 引擎（浏览器
//! V8/JSC、wasmtime、wasmi、Node）拿到同一份产物皆可实例化执行，
//! 宿主以 `rtcl.*` 导入提供运行时（契约全文 = docs/JIT-WASM-GC.md
//! ABI v2；机器真值 = crates/rtcl-jit/src/abi.rs）。
//!
//! 与进程内 JIT（rtcl-jit fast-tier）共享同一 emit 管线；本 crate
//! 只做产物封装（编译 + 清单 + 回环验证），不承载执行机器。

use rtcl_jit::emit;
use rtcl_jit::pass::EmitConfig;
use rtcl_parser::{ByteCode, Compiler, ScriptUnit};
use std::rc::Rc;

pub struct Artifact {
    /// Tcl 语义体（执行引擎按名读帧/重分派需要）。
    pub bc: ByteCode,
    /// emit 管线全产物（bytes + l2/l3 + fast 元数据）。
    pub module: emit::Module,
    /// 产物的 .wasm 字节（module.bytes 的Owned副本，直接写盘）。
    pub bytes: Vec<u8>,
    /// ABI 契约清单（imports/exports/内存/表/提案需求）。
    pub manifest: String,
}

/// 编译 Tcl 脚本 → wasm 产物 + 清单。纯编译，不需要解释器实例
///（emit 管线只消费 ByteCode）。
pub fn compile_script(script: &str) -> Result<Artifact, String> {
    let unit = ScriptUnit::parse(script).map_err(|e| format!("parse: {e:?}"))?;
    // 解释器原生融合超指令展开回规范窗口（JIT 接受集契约，与
    // run_top_with 同款——docs/JIT-OPCODE-DELTA.md）。
    let bc = Compiler::compile_unit(Rc::clone(&unit.source), &unit.commands).unfused();
    let module = emit::compile(&bc, &EmitConfig::default())
        .map_err(|e| format!("emit: {e:?}"))?;
    let bytes = module.bytes.clone();
    let manifest = build_manifest(&bytes);
    Ok(Artifact { bc, module, bytes, manifest })
}

/// 从产物字节解析 imports/exports/内存/表 → 契约清单（wasmparser
/// 解析真值，不依赖手工同步 abi.rs 常量）。
fn build_manifest(bytes: &[u8]) -> String {
    let mut out = String::from("# towasm artifact manifest\n");
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut memory = None::<(u64, Option<u64>)>;
    let mut table = None::<(u64, Option<u64>)>;
    let mut has_funcref_table = false;
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload.expect("wasm payload") {
            wasmparser::Payload::ImportSection(s) => {
                for imp in s.into_imports() {
                    let imp = imp.expect("import");
                    let kind = match imp.ty {
                        wasmparser::TypeRef::Func(_) => "func",
                        wasmparser::TypeRef::Memory(_) => "memory",
                        wasmparser::TypeRef::Global(_) => "global",
                        wasmparser::TypeRef::Table(_) => "table",
                        _ => "other",
                    };
                    imports.push(format!("  {} {}.{}", kind, imp.module, imp.name));
                    if let wasmparser::TypeRef::Table(t) = imp.ty {
                        has_funcref_table = true;
                        table = Some((t.initial, t.maximum));
                    }
                    if let wasmparser::TypeRef::Memory(m) = imp.ty {
                        memory = Some((m.initial, m.maximum));
                    }
                }
            }
            wasmparser::Payload::ExportSection(s) => {
                for exp in s {
                    let exp = exp.expect("export");
                    let kind = match exp.kind {
                        wasmparser::ExternalKind::Func => "func",
                        wasmparser::ExternalKind::Memory => "memory",
                        wasmparser::ExternalKind::Global => "global",
                        wasmparser::ExternalKind::Table => "table",
                        _ => "other",
                    };
                    exports.push(format!("  {} {}", kind, exp.name));
                }
            }
            wasmparser::Payload::MemorySection(s) => {
                for m in s {
                    let m = m.expect("memory");
                    memory = Some((m.initial, m.maximum));
                }
            }
            _ => {}
        }
    }
    out.push_str(&format!("imports({}):\n", imports.len()));
    out.push_str(&imports.join("\n"));
    out.push('\n');
    out.push_str(&format!("exports({}):\n", exports.len()));
    out.push_str(&exports.join("\n"));
    out.push('\n');
    if let Some((min, max)) = memory {
        out.push_str(&format!("memory: min={min} max={}\n", max.map(|m| m.to_string()).unwrap_or("-".into())));
    }
    if let Some((min, max)) = table {
        out.push_str(&format!("table: funcref min={min} max={}\n", max.map(|m| m.to_string()).unwrap_or("-".into())));
    }
    if has_funcref_table {
        out.push_str("proposals: reference-types (funcref table import)\n");
    }
    out.push_str("abi: docs/JIT-WASM-GC.md ABI v2 / crates/rtcl-jit/src/abi.rs\n");
    out.push_str(
        "proc-bodies: runtime-compiled (jiterpreter 模式——本产物仅含顶层单元；\\
proc 体在宿主执行 `proc` 定义命令时编译并挂 fast-tier funcref 表；\\
AOT 模块集 = towasm 路线图)\n",
    );
    out
}
