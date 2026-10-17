//! `assemble` — L3→L4：wasm-encoder 模块组装（types/imports/globals/
//! memory/code）。每个 L3 模板展开为「快车道 + 守卫 + 宿主兜底」的
//! wasm 指令序列（§4 表的落地）。
//!
//! 寄存器布局（run 的 locals）：
//! `[0 .. max_depth) = 值栈槽(i64)`、`max_depth = 出口/空值槽`、
//! `max_depth+1 = SCRATCH(i64)`、`max_depth+2 = EPOCH_HOME(i64)`、
//! `max_depth+3 .. +bregs = 布尔寄存器(i32)`、最后一枚 = FAIL kind(i32)。

use crate::abi::{self, cache, code, import, ty, BinOp};
use crate::ir::{CmpOp, Instr, L2, L3};
use crate::pass::EmitConfig;
use rtcl_ir::bytecode::ByteCode;
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection, MemoryType,
    Module, TypeSection, ValType,
};

const I64: ValType = ValType::I64;
const I32: ValType = ValType::I32;

struct Env<'a> {
    l2: &'a L2,
    scratch: u32,
    epoch_home: u32,
    bool_base: u32,
    kind: u32,
}

impl<'a> Env<'a> {
    fn b(&self, b: u32) -> u32 {
        self.bool_base + b
    }
    fn name_handle(&self, var: u32) -> i32 {
        crate::abi::const_handle(self.l2.vars[var as usize].const_idx) as i32
    }
}

pub fn run(l3: &L3, l2: &L2, _code: &ByteCode, cfg: &EmitConfig) -> Vec<u8> {
    let _ = cfg;
    let nv = l2.max_depth + 1;
    let env = Env {
        l2,
        scratch: nv + 1,
        epoch_home: nv + 2,
        bool_base: nv + 3,
        kind: nv + 3, // + bregs 在下面补
        // kind 实际 = bool_base + bregs —— bregs 由 run() 调用方传入？
    };
    let _ = env;

    let bregs = count_bregs(l3);
    let env = Env {
        l2,
        scratch: nv + 1,
        epoch_home: nv + 2,
        bool_base: nv + 3,
        kind: nv + 3 + bregs,
    };

    let mut body = Function::new(vec![
        (nv + 3, I64), // 值栈 + 出口槽 + SCRATCH + EPOCH_HOME
        (bregs + 1, I32),
    ]);

    emit_node(l3, &env, &mut body);
    body.instruction(&Instruction::End); // 函数终结（wasm-encoder 不自动补）

    // ── sections ─────────────────────────────────────────────────────────
    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![I32]); // RUN
    types.ty().function(vec![I64], vec![I32]); // BOX
    types.ty().function(vec![ValType::F64], vec![I32]); // BOX_FLOAT
    types.ty().function(vec![I32, I64, I64], vec![I64]); // BINARY_OP
    types.ty().function(vec![I32], vec![I64]); // I32_TO_I64
    types.ty().function(vec![I32, I64], vec![I32]); // SET_VAR → 状态
    types.ty().function(vec![I32, I32, I32], vec![I64]); // CALL
    types.ty().function(vec![I64], vec![]); // SET_RESULT
    types.ty().function(vec![], vec![I64]); // REFUEL
    types.ty().function(vec![I32], vec![I32]); // FAIL → kind
    types.ty().function(vec![I32], vec![I32, I64]); // UNBOX_INT
    types.ty().function(vec![I32], vec![I32, ValType::F64]); // UNBOX_FLOAT
    types.ty().function(vec![I32, I64], vec![I64]); // INCR_VAR
    types.ty().function(vec![I32], vec![]); // I32_VOID2 (exit)
    types.ty().function(vec![I64], vec![I64]); // BOOL_TEST

    let mut imports = ImportSection::new();
    let mut_g = |t: ValType| GlobalType { val_type: t, mutable: true, shared: false };
    imports.import("rtcl", "fuel", EntityType::Global(mut_g(I64)));
    imports.import("rtcl", "epoch", EntityType::Global(mut_g(I64)));
    imports.import("rtcl", "caching", EntityType::Global(mut_g(I32)));
    imports.import("rtcl", "unbox_int", EntityType::Function(ty::UNBOX_INT));
    imports.import("rtcl", "unbox_float", EntityType::Function(ty::UNBOX_FLOAT));
    imports.import("rtcl", "box", EntityType::Function(ty::BOX));
    imports.import("rtcl", "box_float", EntityType::Function(ty::BOX_FLOAT));
    imports.import("rtcl", "binary_op", EntityType::Function(ty::BINARY_OP));
    imports.import("rtcl", "get_var", EntityType::Function(ty::I32_TO_I64));
    imports.import("rtcl", "set_var", EntityType::Function(ty::SET_VAR));
    imports.import("rtcl", "call", EntityType::Function(ty::CALL));
    imports.import("rtcl", "set_result", EntityType::Function(ty::SET_RESULT));
    imports.import("rtcl", "refuel", EntityType::Function(ty::REFUEL));
    imports.import("rtcl", "fail", EntityType::Function(ty::FAIL));
    imports.import("rtcl", "bool_test", EntityType::Function(ty::BOOL_TEST));
    imports.import("rtcl", "incr_var", EntityType::Function(ty::INCR_VAR));
    imports.import("rtcl", "push_const", EntityType::Function(ty::I32_TO_I64));
    imports.import("rtcl", "exit", EntityType::Function(ty::I32_VOID2));

    let mut funcs = FunctionSection::new();
    funcs.function(ty::RUN);

    let mut globals = GlobalSection::new();
    // 3 imported globals (fuel/epoch/caching) → cur_site = global index 3。
    globals.global(
        GlobalType { val_type: I32, mutable: true, shared: false },
        &wasm_encoder::ConstExpr::i32_const(-1),
    );

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, import::RUN);
    exports.export("cur_site", ExportKind::Global, 3);

    let mut memory = MemorySection::new();
    memory.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });

    let mut code_section = CodeSection::new();
    code_section.function(&body);

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    module.section(&memory);
    module.section(&globals);
    module.section(&exports);
    module.section(&code_section);
    module.finish()
}

fn count_bregs(l3: &L3) -> u32 {
    // 布尔寄存器数量 = lower 分配数；从 L3 中扫描最大 breg + 1。
    let mut max = 0u32;
    walk(l3, &mut |n| {
        if let L3::If { cond, .. } = n {
            max = max.max(cond.0 + 1);
        }
        if let L3::Instr(Instr::BoolTest { dst, .. }) = n {
            max = max.max(dst.0 + 1);
        }
    });
    max
}

fn walk(n: &L3, f: &mut impl FnMut(&L3)) {
    f(n);
    match n {
        L3::Seq(items) => items.iter().for_each(|i| walk(i, f)),
        L3::If { then_, else_, .. } => {
            walk(then_, f);
            walk(else_, f);
        }
        L3::Loop { body, .. } | L3::Block { body, .. } => walk(body, f),
        _ => {}
    }
}

fn emit_node(n: &L3, e: &Env, f: &mut Function) {
    match n {
        L3::Seq(items) => items.iter().for_each(|i| emit_node(i, e, f)),
        L3::If { cond, then_, else_, .. } => {
            f.instruction(&Instruction::LocalGet(e.b(cond.0)));
            f.instruction(&Instruction::If(BlockType::Empty));
            emit_node(then_, e, f);
            f.instruction(&Instruction::Else);
            emit_node(else_, e, f);
            f.instruction(&Instruction::End);
        }
        L3::Loop { body, .. } => {
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            emit_node(body, e, f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        }
        L3::Block { body, .. } => {
            f.instruction(&Instruction::Block(BlockType::Empty));
            emit_node(body, e, f);
            f.instruction(&Instruction::End);
        }
        L3::Br { depth, .. } => { f.instruction(&Instruction::Br(*depth)); }
        L3::BrIf { depth, cond, when_zero, .. } => {
            f.instruction(&Instruction::LocalGet(e.b(cond.0)));
            if *when_zero {
                f.instruction(&Instruction::I32Eqz);
            }
            f.instruction(&Instruction::BrIf(*depth));
        }
        L3::Instr(i) => emit_instr(i, e, f),
    }
}

fn ld8(f: &mut Function, off: u32) {
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Load8U(MemArg { offset: off as u64, align: 0, memory_index: 0 }));
}
fn st8(f: &mut Function, off: u32, val: i32) {
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(val));
    f.instruction(&Instruction::I32Store8(MemArg { offset: off as u64, align: 0, memory_index: 0 }));
}
fn ld64(f: &mut Function, off: u32) {
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Load(MemArg { offset: off as u64, align: 3, memory_index: 0 }));
}
/// mem[off] = local.get(src)——store 需要 [addr, value] 栈序。
fn st64_local(f: &mut Function, off: u32, src: u32) {
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(src));
    f.instruction(&Instruction::I64Store(MemArg { offset: off as u64, align: 3, memory_index: 0 }));
}

/// 哨兵检查：栈顶（刚 local.set 过的寄存器 reg）== 2 → fail 路径。
fn check_sentinel(e: &Env, f: &mut Function, reg: u32, fail: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
    f.instruction(&Instruction::I64Eq);
    f.instruction(&Instruction::If(BlockType::Empty));
    fail_path(e, f, fail);
    f.instruction(&Instruction::End);
}

/// `rtcl.fail(id)` → i32 kind；kind==0（纯错误）→ return TCL_ERROR。
/// 其余 kind 由 CallSite 模板特判（break/continue 出口）。
fn fail_path(_e: &Env, f: &mut Function, fail: u32) {
    f.instruction(&Instruction::I32Const(fail as i32));
    f.instruction(&Instruction::Call(import::FAIL));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::I32Const(code::ERROR));
    f.instruction(&Instruction::Return);
    f.instruction(&Instruction::End);
    // kind != 0：值已被 eqz 消费；仅 CallSite 语境可达（其自带分派）。
}

/// 值寄存器压 retag：`local.get reg; shl 1; or 1; local.set reg`。
fn retag_in_place(f: &mut Function, reg: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Shl);
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Or);
    f.instruction(&Instruction::LocalSet(reg));
}

/// SCRATCH = local.get(reg) >> 62；结果 in {0,-1} 时继续（br_if 跳过 slow）。
/// 调用形态：`block $ok { <range ok> br_if $ok_inner... }`——这里给
/// 「range 不 ok 则 fallthrough 到 slow」的复合片段：跳转目标 = 1（外层
/// slow block 的内层 ok block 由调用方保证）。约定：调用方提供
/// `block $ok { … }` 包裹快车道，range-bad 时 `br 1`（跳到 slow）。
fn range_guard_br_slow(f: &mut Function, reg: u32, scratch: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(62));
    f.instruction(&Instruction::I64ShrS);
    f.instruction(&Instruction::LocalSet(scratch));
    // ok = (t == 0) | (t == -1)；!ok → br 1（slow）
    f.instruction(&Instruction::LocalGet(scratch));
    f.instruction(&Instruction::I64Eqz);
    f.instruction(&Instruction::LocalGet(scratch));
    f.instruction(&Instruction::I64Const(-1));
    f.instruction(&Instruction::I64Eq);
    f.instruction(&Instruction::I32Or);
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::BrIf(1));
}

fn imm_guard_br_slow(f: &mut Function, reg: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64And);
    f.instruction(&Instruction::I64Eqz);
    f.instruction(&Instruction::BrIf(1));
}

/// retag( (i32 bool) → tagged i64 )：值在 i32 栈顶。
fn tag_bool(f: &mut Function, dst: u32) {
    f.instruction(&Instruction::I64ExtendI32U);
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Shl);
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Or);
    f.instruction(&Instruction::LocalSet(dst));
}

/// 回边 fuel：自减、耗尽调 `rtcl.refuel`（宿主充值 fuel global；
/// 返回 0=继续 / 2=预算耗尽哨兵）。
fn emit_fuel(e: &Env, f: &mut Function, fail: u32) {
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64Sub);
    f.instruction(&Instruction::GlobalSet(0));
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I64GeS);
    f.instruction(&Instruction::If(BlockType::Empty));
    // fuel 充足：无操作。
    f.instruction(&Instruction::Else);
    f.instruction(&Instruction::Call(import::REFUEL));
    f.instruction(&Instruction::LocalSet(e.scratch));
    check_sentinel(e, f, e.scratch, fail);
    f.instruction(&Instruction::End);
}

fn emit_instr(i: &Instr, e: &Env, f: &mut Function) {
    match i {
        Instr::Const { dst, bits, .. } => {
            f.instruction(&Instruction::I64Const(*bits));
            f.instruction(&Instruction::LocalSet(dst.0));
        }
        Instr::Copy { dst, src } => {
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::LocalSet(dst.0));
        }
        Instr::BoxFloat { dst, v } => {
            f.instruction(&Instruction::F64Const((*v).into()));
            f.instruction(&Instruction::Call(import::BOX_FLOAT));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(dst.0));
        }
        Instr::LoadVar { dst, var, fail } => {
            let var = *var;
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // host
            f.instruction(&Instruction::GlobalGet(2)); // caching
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0)); // → host
            // 值槽 0 = 未缓存（合法值恒非 0：imm 为奇、句柄 ≥4）。
            ld64(f, vals);
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::BrIf(0)); // absent → host
            ld64(f, vals);
            f.instruction(&Instruction::LocalSet(dst.0));
            f.instruction(&Instruction::Br(1)); // → done
            f.instruction(&Instruction::End); // host:
            f.instruction(&Instruction::I32Const(e.name_handle(var)));
            f.instruction(&Instruction::Call(import::GET_VAR));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
            st64_local(f, vals, dst.0);
            st8(f, state, 1);
            f.instruction(&Instruction::End); // done
        }
        Instr::StoreVar { src, var, keep: _, fail } => {
            let var = *var;
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // host
            f.instruction(&Instruction::GlobalGet(2));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0));
            st64_local(f, vals, src.0);
            st8(f, state, 2);
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // host:
            f.instruction(&Instruction::I32Const(e.name_handle(var)));
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::SET_VAR));
            // set_var 返回 i32 状态（ABI 加法：错误信令），0=ok。
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            fail_path(e, f, *fail);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End); // done
        }
        Instr::IncrVar { dst, var, amount, fail } => {
            let var = *var;
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // host
            f.instruction(&Instruction::GlobalGet(2));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0));
            ld64(f, vals); // 0 = 未缓存
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::BrIf(0));
            ld64(f, vals);
            f.instruction(&Instruction::LocalSet(dst.0));
            imm_guard_br_slow(f, dst.0);
            // n = untag + amount
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Const(*amount));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(dst.0));
            range_guard_br_slow(f, dst.0, e.scratch);
            retag_in_place(f, dst.0);
            st64_local(f, vals, dst.0);
            st8(f, state, 2);
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // host:
            f.instruction(&Instruction::I32Const(e.name_handle(var)));
            f.instruction(&Instruction::I64Const(crate::abi::tag_imm(*amount)));
            f.instruction(&Instruction::Call(import::INCR_VAR));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
            f.instruction(&Instruction::End); // done
        }
        Instr::CallSite { dst, cmd_id, argc, fail, site, cont_back, brk_depth, cont_depth, .. } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(*cmd_id as i32));
            // argv_base_h 槽位在 M1' 传站点索引（宿主按站点原文 eval 路由；
            // §3 的 argv 语义留给 M2 的参数直传）。
            f.instruction(&Instruction::I32Const(*site as i32));
            f.instruction(&Instruction::I32Const(*argc as i32));
            f.instruction(&Instruction::Call(import::CALL));
            f.instruction(&Instruction::LocalSet(dst.0));
            // 哨兵 → rtcl.fail(id) 返回 kind：
            //   0=纯错误 → return TCL_ERROR；1=break(level1) → br 内层
            //   循环 brk；2=continue(level1) → br cont（回边先扣 fuel）；
            //   3/4=break/continue(level>1) → return BREAK/CONTINUE。
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(*fail as i32));
            f.instruction(&Instruction::Call(import::FAIL));
            f.instruction(&Instruction::LocalSet(e.kind));
            if let Some(d) = brk_depth {
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::BrIf(d + 1));
            }
            if let Some(d) = cont_depth {
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(2));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                if *cont_back {
                    emit_fuel(e, f, *fail);
                }
                f.instruction(&Instruction::Br(d + 2));
                f.instruction(&Instruction::End);
            }
            for (k, c) in [(3, code::BREAK), (4, code::CONTINUE)] {
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(k));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::I32Const(c));
                f.instruction(&Instruction::Return);
                f.instruction(&Instruction::End);
            }
            f.instruction(&Instruction::I32Const(code::ERROR));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
        }
        Instr::Arith { kind, dst, a, b, fail } => emit_arith(*kind, dst.0, a.0, b.0, *fail, e, f),
        Instr::Cmp { cc, dst, a, b, fail } => {
            // §4：tagged word 单调，免 untag 直接比较。
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // slow
            imm_guard_br_slow(f, a.0);
            imm_guard_br_slow(f, b.0);
            f.instruction(&Instruction::LocalGet(a.0));
            f.instruction(&Instruction::LocalGet(b.0));
            f.instruction(match cc {
                CmpOp::Lt => &Instruction::I64LtS,
                CmpOp::Gt => &Instruction::I64GtS,
                CmpOp::Le => &Instruction::I64LeS,
                CmpOp::Ge => &Instruction::I64GeS,
            });
            tag_bool(f, dst.0);
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // slow:
            let op = match cc {
                CmpOp::Lt => BinOp::Lt,
                CmpOp::Gt => BinOp::Gt,
                CmpOp::Le => BinOp::Le,
                CmpOp::Ge => BinOp::Ge,
            };
            call_binary_op(op, a.0, b.0, dst.0, *fail, e, f);
            f.instruction(&Instruction::End); // done
        }
        Instr::Unary { kind, dst, a, fail } => emit_unary(*kind, dst.0, a.0, *fail, e, f),
        Instr::BoolOfBool { dst, v } => {
            // tagged 0/1 → bool：v != 1（tag(0)=1）。
            f.instruction(&Instruction::LocalGet(v.0));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Ne);
            f.instruction(&Instruction::LocalSet(e.b(dst.0)));
        }
        Instr::BoolTest { dst, v, fail } => {
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // host
            imm_guard_br_slow(f, v.0);
            // tagged imm: bool = v != 1
            f.instruction(&Instruction::LocalGet(v.0));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Ne);
            f.instruction(&Instruction::LocalSet(e.b(dst.0)));
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // host:
            f.instruction(&Instruction::LocalGet(v.0));
            f.instruction(&Instruction::Call(import::BOOL_TEST));
            f.instruction(&Instruction::LocalSet(e.scratch));
            check_sentinel(e, f, e.scratch, *fail);
            // host 返回 tagged 0/1：tag(1)=3 → bool
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::I64Const(3));
            f.instruction(&Instruction::I64Eq);
            f.instruction(&Instruction::LocalSet(e.b(dst.0)));
            f.instruction(&Instruction::End); // done
        }
        Instr::SetSite(k) => {
            f.instruction(&Instruction::I32Const(*k as i32));
            f.instruction(&Instruction::GlobalSet(3));
        }
        Instr::SetResult { src } => {
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::SET_RESULT));
        }
        Instr::ReturnCode { code } => {
            f.instruction(&Instruction::I32Const(*code));
            f.instruction(&Instruction::Return);
        }
        Instr::Fail { fail } => {
            f.instruction(&Instruction::I32Const(*fail as i32));
            f.instruction(&Instruction::Call(import::FAIL));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::I32Const(code::ERROR));
            f.instruction(&Instruction::Return);
        }
        Instr::Exit { code } => {
            f.instruction(&Instruction::I32Const(*code));
            f.instruction(&Instruction::Call(import::EXIT));
            f.instruction(&Instruction::I32Const(code::EXIT));
            f.instruction(&Instruction::Return);
        }
        Instr::BackedgeFuel { fail } => emit_fuel(e, f, *fail),
        Instr::EpochCheck { at_entry } => {
            if *at_entry {
                f.instruction(&Instruction::GlobalGet(1));
                f.instruction(&Instruction::LocalSet(e.epoch_home));
            } else {
                f.instruction(&Instruction::GlobalGet(1));
                f.instruction(&Instruction::LocalGet(e.epoch_home));
                f.instruction(&Instruction::I64Ne);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::I32Const(code::STALE));
                f.instruction(&Instruction::Return);
                f.instruction(&Instruction::End);
            }
        }
    }
}

fn call_binary_op(op: BinOp, a: u32, b: u32, dst: u32, fail: u32, e: &Env, f: &mut Function) {
    f.instruction(&Instruction::I32Const(op as i32));
    f.instruction(&Instruction::LocalGet(a));
    f.instruction(&Instruction::LocalGet(b));
    f.instruction(&Instruction::Call(import::BINARY_OP));
    f.instruction(&Instruction::LocalSet(dst));
    check_sentinel(e, f, dst, fail);
}

/// 缓存 flush：dirty 槽写回宿主（host set_var 全语义含 trace 触发）。
fn emit_flush(e: &Env, f: &mut Function, fail: u32) {
    if e.l2.vars.is_empty() {
        return;
    }
    f.instruction(&Instruction::GlobalGet(2));
    f.instruction(&Instruction::If(BlockType::Empty));
    for (k, v) in e.l2.vars.iter().enumerate() {
        let vals = cache::VALUES + k as u32 * 8;
        let state = cache::STATE + k as u32;
        let name_h = crate::abi::const_handle(v.const_idx) as i32;
        ld8(f, state);
        f.instruction(&Instruction::I32Const(2));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(name_h));
        ld64(f, vals);
        f.instruction(&Instruction::Call(import::SET_VAR));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        fail_path(e, f, fail);
        f.instruction(&Instruction::End);
        st8(f, state, 1);
        f.instruction(&Instruction::End);
    }
    f.instruction(&Instruction::End);
}

fn emit_unary(kind: BinOp, dst: u32, a: u32, fail: u32, e: &Env, f: &mut Function) {
    f.instruction(&Instruction::Block(BlockType::Empty)); // done
    f.instruction(&Instruction::Block(BlockType::Empty)); // slow
    imm_guard_br_slow(f, a);
    match kind {
        BinOp::Neg => {
            // 0 - untag(a)
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(dst));
            range_guard_br_slow(f, dst, e.scratch);
        }
        BinOp::Not => {
            // bool = a != 1（tagged imm）；!bool
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Ne);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(dst));
        }
        BinOp::BitNot => {
            // -1 ^ untag(a)
            f.instruction(&Instruction::I64Const(-1));
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Xor);
            f.instruction(&Instruction::LocalSet(dst));
            range_guard_br_slow(f, dst, e.scratch);
        }
        _ => unreachable!("unary kind"),
    }
    if kind != BinOp::Not {
        retag_in_place(f, dst);
    } else {
        // dst 已是未 retag 的 0/1 —— 统一 retag。
        retag_in_place(f, dst);
    }
    f.instruction(&Instruction::Br(1));
    f.instruction(&Instruction::End); // slow:
    call_binary_op(kind, a, e.scratch /* b 任意 */, dst, fail, e, f);
    f.instruction(&Instruction::End); // done
}

fn emit_arith(kind: BinOp, dst: u32, a: u32, b: u32, fail: u32, e: &Env, f: &mut Function) {
    // 全走宿主的 §4 类别：div/mod（floor 语义与除零）、pow、字符串比较。
    if matches!(kind, BinOp::Div | BinOp::Mod | BinOp::Pow | BinOp::StrEq | BinOp::StrNe) {
        call_binary_op(kind, a, b, dst, fail, e, f);
        return;
    }
    f.instruction(&Instruction::Block(BlockType::Empty)); // done
    f.instruction(&Instruction::Block(BlockType::Empty)); // slow
    imm_guard_br_slow(f, a);
    imm_guard_br_slow(f, b);
    match kind {
        BinOp::Add | BinOp::Sub | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalGet(b));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(match kind {
                BinOp::Add => &Instruction::I64Add,
                BinOp::Sub => &Instruction::I64Sub,
                BinOp::BitAnd => &Instruction::I64And,
                BinOp::BitOr => &Instruction::I64Or,
                _ => &Instruction::I64Xor,
            });
            f.instruction(&Instruction::LocalSet(dst));
            range_guard_br_slow(f, dst, e.scratch);
        }
        BinOp::Mul => {
            // §4：高 32 位符号扩展检查（|a|,|b| < 2^31）→ 积 < 2^62。
            for r in [a, b] {
                f.instruction(&Instruction::LocalGet(r));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64ShrS);
                f.instruction(&Instruction::I64Const(31));
                f.instruction(&Instruction::I64ShrS);
                f.instruction(&Instruction::LocalSet(e.scratch));
                f.instruction(&Instruction::LocalGet(e.scratch));
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::LocalGet(e.scratch));
                f.instruction(&Instruction::I64Const(-1));
                f.instruction(&Instruction::I64Eq);
                f.instruction(&Instruction::I32Or);
                f.instruction(&Instruction::I32Eqz);
                f.instruction(&Instruction::BrIf(1));
            }
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalGet(b));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::LocalSet(dst));
        }
        BinOp::Shl | BinOp::Shr => {
            // 移位数 b_u ∈ [0, 63]（负数/超宽 → 宿主精确语义）。
            for k in [0i64, 63] {
                f.instruction(&Instruction::LocalGet(b));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64ShrS);
                f.instruction(&Instruction::I64Const(k));
                f.instruction(if k == 0 {
                    &Instruction::I64LtS
                } else {
                    &Instruction::I64GtS
                });
            }
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Br(1)); // → slow
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalGet(b));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(if kind == BinOp::Shl {
                &Instruction::I64Shl
            } else {
                &Instruction::I64ShrS
            });
            f.instruction(&Instruction::LocalSet(dst));
            if kind == BinOp::Shl {
                range_guard_br_slow(f, dst, e.scratch);
            }
        }
        BinOp::Eq | BinOp::Ne => {
            // §4：tagged 直接比较。
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::LocalGet(b));
            f.instruction(if kind == BinOp::Eq {
                &Instruction::I64Eq
            } else {
                &Instruction::I64Ne
            });
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(dst));
        }
        BinOp::And | BinOp::Or => {
            // 双立即数：bool = v != 1（tagged imm）。
            for r in [a, b] {
                f.instruction(&Instruction::LocalGet(r));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64Ne);
            }
            f.instruction(if kind == BinOp::And {
                &Instruction::I32And
            } else {
                &Instruction::I32Or
            });
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(dst));
        }
        _ => unreachable!("arith kind"),
    }
    retag_in_place(f, dst);
    f.instruction(&Instruction::Br(1));
    f.instruction(&Instruction::End); // slow:
    call_binary_op(kind, a, b, dst, fail, e, f);
    f.instruction(&Instruction::End); // done
}
