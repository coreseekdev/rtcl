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
    GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection, MemoryType, RefType,
    TableType,
    Module, TypeSection, ValType,
};

const I64: ValType = ValType::I64;
const I32: ValType = ValType::I32;

struct Env<'a> {
    l2: &'a L2,
    /// foreach 计数器（i/iters 对）的 local 基址。
    foreach_base: u32,
    scratch: u32,
    epoch_home: u32,
    bool_base: u32,
    kind: u32,
    /// fast-tier（proc_main 发射中）：Some(ctx) 时——LoadVar(参数)→
    /// local.get；SetResult 免宿主直记寄存器；fail/出口返回
    /// (i64 哨兵, i32 rc) 多值；cur_site global 下标 = 4（多导入
    /// cur_proc 一个）。
    fast: Option<FastCtx>,
}

struct FastCtx {
    /// var（l2.vars 下标）→ 参数 slot（per-depth 参数区偏移 = slot*8）。
    param_slot: std::collections::HashMap<u32, u32>,
    /// SetResult 记录的结果寄存器（ReturnCode 出口读取）。
    res_reg: std::cell::Cell<Option<u32>>,
    /// 编译期 tier1 epoch（CallSite 槽缓存失效基准）。
    epoch: u64,
}

impl Env<'_> {
    /// fast 模式下的 cur_site global 下标（4 个导入 global）。
    fn site_global(&self) -> u32 {
        if self.fast.is_some() { 6 } else { 3 }
    }
}

impl<'a> Env<'a> {
    fn b(&self, b: u32) -> u32 {
        self.bool_base + b
    }
    fn name_handle(&self, var: u32) -> i32 {
        crate::abi::const_handle(self.l2.vars[var as usize].const_idx) as i32
    }
}

pub fn run(l3: &L3, l2: &L2, code: &ByteCode, cfg: &EmitConfig) -> (Vec<u8>, Option<crate::emit::FastMeta>) {
    let _ = cfg;
    let nv = l2.max_depth + 1;
    let bregs = count_bregs(l3);
    let nf = count_foreach(l3);
    let foreach_base = nv + 3;
    let nparams = fast_gate(l2, l3, code);
    let env = Env {
        l2,
        foreach_base,
        scratch: nv + 1,
        epoch_home: nv + 2,
        bool_base: foreach_base + 2 * nf,
        kind: foreach_base + 2 * nf + bregs,
        fast: None,
    };

    let mut body = Function::new(vec![
        (nv + 3 + 2 * nf, I64), // 值栈 + 出口 + SCRATCH + EPOCH + foreach 对
        (bregs + 1, I32),
    ]);

    // ── 序言：cache 区清零 + caching=1（池重置的 wasm 侧承担）──────
    // 池命中的宿主重置此前每调用写 512+vars 字节 + 1 次 global.set
    //（~120ns）；搬进序言后每 var 2 条 store（~2ns）+ 1 条 global.set，
    // 且宿主 pool_reset 瘦身为纯 reset_for_reuse。语义：清零发生在
    // 任何 LoadVar 之前——cache 契约「caching=1 ⇒ VALUES 槽新鲜」在
    // 运行起点成立；运行中的 impure 路由仍以 caching=0 失效整窗。
    // 变量数 = l2.vars.len()（名字键变量表）——nv 是值栈寄存器数，
    // 两者无关；少清会让池复用读到陈旧缓存（namespace-old-7.10 实证）。
    let nvars = env.l2.vars.len() as u32;
    // 地址风格 = 全文件约定：I32Const(0) + MemArg.offset（有效地址 =
    // addr + offset；双写会让清零落到 2×off——namespace-old-7.10 实证）。
    for var in 0..nvars {
        let vals = cache::VALUES + var * 8;
        let state = cache::STATE + var;
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::I64Const(0));
        body.instruction(&Instruction::I64Store(MemArg {
            offset: vals as u64,
            align: 3,
            memory_index: 0,
        }));
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::I32Store8(MemArg {
            offset: state as u64,
            align: 0,
            memory_index: 0,
        }));
    }
    body.instruction(&Instruction::I32Const(1));
    body.instruction(&Instruction::GlobalSet(2)); // caching = 1

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
    // CALL type (i32,i32,i32)->i64 reused for call_argv — no new type needed
    types.ty().function(vec![I64], vec![I64]); // BOOL_TEST
    types.ty().function(vec![I32, I32], vec![I64]); // CONCAT_TY
    types.ty().function(vec![I32, I32, I32], vec![I64]); // FOREACH_START
    types.ty().function(vec![I32, I32], vec![I64]); // FOREACH_BIND
    types.ty().function(vec![I64], vec![]); // FOREACH_COLLECT
    types.ty().function(vec![], vec![I64]); // FOREACH_END_TY
    types.ty().function(vec![I64], vec![I64]); // I64_TO_I64 (eval_script/eval_expr)
    types.ty().function(vec![I32, I32], vec![I64]); // CONCAT
    // fast-tier：多值 proc_main（值 + rc）与无参 fast_impl（run 同布局）。
    types.ty().function(vec![I32, I32, I32], vec![I64, I32]); // PROC_MAIN
    types.ty().function(vec![], vec![I64, I32]); // PROC_IMPL

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
    imports.import("rtcl", "exit", EntityType::Function(ty::I32_VOID));
    imports.import("rtcl", "concat_n", EntityType::Function(ty::CONCAT_TY));
    imports.import("rtcl", "foreach_start", EntityType::Function(ty::FOREACH_START));
    imports.import("rtcl", "foreach_bind", EntityType::Function(ty::FOREACH_BIND));
    imports.import("rtcl", "foreach_collect", EntityType::Function(ty::FOREACH_COLLECT));
    imports.import("rtcl", "foreach_end", EntityType::Function(ty::FOREACH_END_TY));
    imports.import("rtcl", "eval_script", EntityType::Function(ty::I64_TO_I64));
    imports.import("rtcl", "eval_expr", EntityType::Function(ty::I64_TO_I64));
    imports.import("rtcl", "lappend_var", EntityType::Function(ty::INCR_VAR));
    imports.import("rtcl", "call_argv", EntityType::Function(ty::CALL));
    // fast-tier 冷路径导入（常规模块也导入——linker 定义 stub 保持
    // 函数索引全局一致：import::RUN/COUNT）。
    imports.import("rtcl", "resolve_proc", EntityType::Function(ty::CALL));
    imports.import("rtcl", "proc_exit_frame", EntityType::Function(ty::I32_VOID));
    if nparams.is_some() {
        // fast 专属导入：共享 memory、funcref 表、cur_proc/
        // site_cache_base/fast_depth 三个可变 global（cur_site=6）。
        imports.import("rtcl", "memory", EntityType::Memory(MemoryType {
            minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None,
        }));
        imports.import("rtcl", "funs", EntityType::Table(TableType {
            element_type: RefType::FUNCREF, minimum: 256, maximum: None, shared: false, table64: false,
        }));
        imports.import("rtcl", "cur_proc", EntityType::Global(GlobalType {
            val_type: I32, mutable: true, shared: false }));
        imports.import("rtcl", "site_cache_base", EntityType::Global(GlobalType {
            val_type: I32, mutable: true, shared: false }));
        imports.import("rtcl", "fast_depth", EntityType::Global(GlobalType {
            val_type: I32, mutable: true, shared: false }));
    }

    // ── fast-tier：fast_impl + proc_main 薄 wrapper ──────────────────
    // 三函数结构：proc_main(3 个 i32 参数) 占 local 0-2 会推移值栈寄存器
    // 布局（kind/scratch/epoch 全错位）——所以 fast 体放**无参**的
    // fast_impl（局部布局与 run 完全一致，零移位）；proc_main 只做
    // cur_proc 保存/设置、fast_depth++、参数区拷贝（per-depth 切片，
    // 递归互不覆盖）、fuel、尾调 fast_impl、出栈恢复。
    let mut fast_meta = None;
    let mut fast_impl: Option<Function> = None;
    let mut fast_wrap: Option<Function> = None;
    if let Some(np) = nparams {
        // 参数 var 映射：l2.vars 中 const_idx ∈ [const_base, const_base+np)
        let mut param_slot = std::collections::HashMap::new();
        let consts_len2 = code.constants().len() as u32;
        for (vi, v) in l2.vars.iter().enumerate() {
            if let Some(slot) = v.const_idx.checked_sub(consts_len2) {
                if (slot as usize) < np {
                    param_slot.insert(vi as u32, slot);
                }
            }
        }
        let fenv = Env {
            l2,
            foreach_base,
            scratch: nv + 1,
            epoch_home: nv + 2,
            bool_base: foreach_base + 2 * nf,
            kind: foreach_base + 2 * nf + bregs,
            fast: Some(FastCtx {
                param_slot,
                res_reg: std::cell::Cell::new(None),
                epoch: code.epoch,
            }),
        };
        // fast_impl：局部 = run 布局（无参数移位）
        let mut fb = Function::new(vec![
            (nv + 3 + 2 * nf, I64),
            (bregs + 1, I32),
        ]);
        emit_node(l3, &fenv, &mut fb);
        fb.instruction(&Instruction::End);
        fast_impl = Some(fb);

        // proc_main wrapper：locals = 2 个 i32（saved_cur, param_base）
        let mut fw = Function::new(vec![(2, I32)]);
        // cur_proc 保存/设置
        fw.instruction(&Instruction::GlobalGet(3));
        fw.instruction(&Instruction::LocalSet(3));
        fw.instruction(&Instruction::LocalGet(0));
        fw.instruction(&Instruction::GlobalSet(3));
        // fast_depth++（错误帧内外层判别 + 参数区切片下标）
        fw.instruction(&Instruction::GlobalGet(5));
        fw.instruction(&Instruction::I32Const(1));
        fw.instruction(&Instruction::I32Add);
        fw.instruction(&Instruction::GlobalSet(5));
        // param_base = PARAMS + depth * PARAM_STRIDE
        fw.instruction(&Instruction::I32Const(cache::PARAMS as i32));
        fw.instruction(&Instruction::GlobalGet(5));
        fw.instruction(&Instruction::I32Const(cache::PARAM_STRIDE as i32));
        fw.instruction(&Instruction::I32Mul);
        fw.instruction(&Instruction::I32Add);
        fw.instruction(&Instruction::LocalSet(4));
        // 拷贝实参（跳过 argv[0]——SCRATCH 暂存的是 dispatch_values 形状，
        // args[0] = 命令名）：mem[param_base + i*8] = mem[argv + (i+1)*8]
        for i in 0..np as u32 {
            fw.instruction(&Instruction::LocalGet(4));
            fw.instruction(&Instruction::I32Const((i * 8) as i32));
            fw.instruction(&Instruction::I32Add);
            fw.instruction(&Instruction::LocalGet(2));
            fw.instruction(&Instruction::I32Const(((i + 1) * 8) as i32));
            fw.instruction(&Instruction::I32Add);
            fw.instruction(&Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
            fw.instruction(&Instruction::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
        }
        // fuel：扣 1，空 → refuel。成功（返回 0，fuel 已充值 chunk）
        // **继续执行**；失败（哨兵，pending 已置）→ 恢复栈 + (哨兵,
        // ERROR)。此前无条件返回 ERROR——fib 在第 ~1024 次调用（首个
        // chunk 耗尽）全体报「failed without pending」的根因。
        fw.instruction(&Instruction::GlobalGet(0));
        fw.instruction(&Instruction::I64Const(1));
        fw.instruction(&Instruction::I64Sub);
        fw.instruction(&Instruction::GlobalSet(0));
        fw.instruction(&Instruction::GlobalGet(0));
        fw.instruction(&Instruction::I64Const(0));
        fw.instruction(&Instruction::I64LtS);
        fw.instruction(&Instruction::If(BlockType::Empty));
        fw.instruction(&Instruction::Call(import::REFUEL));
        fw.instruction(&Instruction::I64Const(0));
        fw.instruction(&Instruction::I64Ne);
        fw.instruction(&Instruction::If(BlockType::Empty));
        fw.instruction(&Instruction::GlobalGet(5));
        fw.instruction(&Instruction::I32Const(1));
        fw.instruction(&Instruction::I32Sub);
        fw.instruction(&Instruction::GlobalSet(5));
        fw.instruction(&Instruction::LocalGet(3));
        fw.instruction(&Instruction::GlobalSet(3));
        fw.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
        fw.instruction(&Instruction::I32Const(code::ERROR));
        fw.instruction(&Instruction::Return);
        fw.instruction(&Instruction::End);
        fw.instruction(&Instruction::End);
        // 尾调 fast_impl → [i64, i32]；恢复栈；透传返回
        fw.instruction(&Instruction::Call(import::FAST_IMPL));
        fw.instruction(&Instruction::GlobalGet(5));
        fw.instruction(&Instruction::I32Const(1));
        fw.instruction(&Instruction::I32Sub);
        fw.instruction(&Instruction::GlobalSet(5));
        fw.instruction(&Instruction::LocalGet(3));
        fw.instruction(&Instruction::GlobalSet(3));
        fw.instruction(&Instruction::Return);
        fw.instruction(&Instruction::End);
        fast_wrap = Some(fw);

        fast_meta = Some(crate::emit::FastMeta { nparams: np, epoch: code.epoch });
    }

    let mut funcs = FunctionSection::new();
    funcs.function(ty::RUN);
    let has_fast = fast_impl.is_some();
    if has_fast {
        funcs.function(ty::PROC_IMPL);
        funcs.function(ty::PROC_MAIN);
    }

    let mut globals = GlobalSection::new();
    // 导入 global 数：常规 3（fuel/epoch/caching）→ cur_site=3；
    // fast 4（+cur_proc）→ cur_site=4。
    globals.global(
        GlobalType { val_type: I32, mutable: true, shared: false },
        &wasm_encoder::ConstExpr::i32_const(-1),
    );

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, import::RUN);
    exports.export("cur_site", ExportKind::Global, if has_fast { 6 } else { 3 });
    exports.export("memory", ExportKind::Memory, 0);
    if has_fast {
        exports.export("proc_main", ExportKind::Func, import::FAST_IMPL + 1);
        // re-export 导入的 cur_proc：宿主闭包经 Caller::get_export 路由
        // per-proc 表——get_export 只查 export 段（import 不可见），
        // 此前 cur 恒 0：单 fast proc 时 hosts[0] 恰好正确（fib 全对
        // 的假象），多 proc 即交叉污染（add 的 arena 把 "uplevel" 句柄
        // 解成 "a"——m3 实证 words 同、name 异）。
        exports.export("cur_proc", ExportKind::Global, 3);
    }

    let mut memory = MemorySection::new();
    if nparams.is_none() {
        // fast 模块导入共享 memory（"rtcl" "memory"）——不自带。
        memory.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    }

    let mut code_section = CodeSection::new();
    code_section.function(&body);
    if let Some(fb) = &fast_impl {
        code_section.function(fb);
    }
    if let Some(fw) = &fast_wrap {
        code_section.function(fw);
    }

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    if nparams.is_none() {
        module.section(&memory);
    }
    module.section(&globals);
    module.section(&exports);
    module.section(&code_section);
    (module.finish(), fast_meta)
}

/// fast-tier 形状门（v1，MoonBit 式「热路径零宿主过渡」接受集）：
/// - params_aligned（精确参数个数 + slot 对齐种子）
/// - 所有 vars 都是参数（vars ⊆ params：cache 区协议不适用，全程
///   wasm local）
/// - 无循环/回边（BREAK/CONTINUE 出口协议未定义）、无 foreach
/// - 无宿主路由 op（EvalScript/EvalExpr/LappendVar/Concat/Foreach*——
///   需要 interp 帧在场）；Call/DynCall 允许（直呼 + 回落双分支）
fn fast_gate(l2: &L2, l3: &L3, code: &ByteCode) -> Option<usize> {
    if let Some(r) = fast_gate_inner(l2, l3, code) {
        return Some(r);
    }
    None
}

fn fast_gate_inner(l2: &L2, l3: &L3, code: &ByteCode) -> Option<usize> {
    use crate::ir::Instr;
    let np = code.params_aligned?;
    // VarSlot.const_idx 是**池相对**索引（name_handle 才加 CONST_BASE）——
    // local 伪常量段 = [consts_len, consts_len+locals)。此前误减
    // l2.const_base（=3）致 fib 的参数 n 被判非常量而拒绝。
    let consts_len = code.constants().len() as u32;
    for v in &l2.vars {
        let slot = v.const_idx.checked_sub(consts_len)?;
        if slot as usize >= np {
            return None;
        }
    }
    // site-cache 段容量（SITE_STRIDE/4 = 256 槽）：超长 proc 不 fast 化。
    if code.sites.len() > (crate::abi::cache::SITE_STRIDE / 4) as usize {
        return None;
    }
    if !l2.backedges.is_empty() {
        return None;
    }
    let mut bad: Option<&'static str> = None;
    let mut has_loop = false;
    walk(l3, &mut |n| {
        if matches!(n, L3::Loop { .. }) {
            has_loop = true;
        }
        let L3::Instr(i) = n else { return };
        let b = match i {
            Instr::EvalScript { .. } => Some("eval_script"),
            Instr::EvalExpr { .. } => Some("eval_expr"),
            Instr::LappendVar { .. } => Some("lappend"),
            Instr::Concat { .. } => Some("concat"),
            Instr::ForeachStart { .. }
            | Instr::ForeachBind { .. }
            | Instr::ForeachStep { .. }
            | Instr::ForeachEnd { .. }
            | Instr::ForeachCollect { .. } => Some("foreach"),
            Instr::BackedgeFuel { .. } => Some("backedge"),
            Instr::StoreVar { .. } | Instr::IncrVar { .. } => Some("store/incr"),
            _ => None,
        };
        if b.is_some() && bad.is_none() {
            bad = b;
        }
    });
    if bad.is_some() || has_loop {
        return None;
    }
    let _ = (&bad, has_loop);
    Some(np)
}

fn count_foreach(l3: &L3) -> u32 {
    let mut max = 0u32;
    walk(l3, &mut |n| {
        let L3::Instr(i) = n else { return };
        let c = match i {
            Instr::ForeachStart { counter, .. } | Instr::ForeachBind { counter, .. }
            | Instr::ForeachStep { counter, .. } => *counter + 1,
            _ => 0,
        };
        max = max.max(c);
    });
    max
}

fn count_bregs(l3: &L3) -> u32 {
    // 布尔寄存器数量 = lower 分配数；从 L3 中扫描最大 breg + 1。
    let mut max = 0u32;
    walk(l3, &mut |n| {
        if let L3::If { cond, .. } = n {
            max = max.max(cond.0 + 1);
        }
        if let L3::BrIf { cond, .. } = n {
            max = max.max(cond.0 + 1);
        }
        if let L3::Instr(
            Instr::BoolTest { dst, .. }
            | Instr::ForeachEntry { b: dst, .. }
            | Instr::ForeachStep { b: dst, .. },
        ) = n
        {
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
/// i32 常量入栈（地址计算用）。
fn f_const(f: &mut Function, v: i32) {
    f.instruction(&Instruction::I32Const(v));
}

fn ld64(f: &mut Function, off: u32) {
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Load(MemArg { offset: off as u64, align: 3, memory_index: 0 }));
}
/// 站点原文 eval 回退开关（启动读一次；CallProc 直连的逐 shape 对照）。
pub fn site_eval_fallback() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static FLAG: AtomicBool = AtomicBool::new(false);
    static INIT: AtomicBool = AtomicBool::new(false);
    if !INIT.swap(true, Ordering::Relaxed) {
        FLAG.store(crate::env_enabled("RTCL_JIT_SITE_EVAL"), Ordering::Relaxed);
    }
    FLAG.load(Ordering::Relaxed)
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

/// fast 模式的多值返回尾：(i64 哨兵, i32 rc)。
fn ret_fast_sentinel(f: &mut Function, rc: i32) {
    f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
    f.instruction(&Instruction::I32Const(rc));
    f.instruction(&Instruction::Return);
}

/// fast 模式出口：depth--（fast_depth global）后按模式返回。
fn fast_depth_dec(f: &mut Function) {
    f.instruction(&Instruction::GlobalGet(5));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::GlobalSet(5));
}

/// `rtcl.fail(id)` → i32 kind；kind==0（纯错误）→ return TCL_ERROR。
/// 其余 kind 由 CallSite 模板特判（break/continue 出口）。
fn fail_path(e: &Env, f: &mut Function, fail: u32) {
    f.instruction(&Instruction::I32Const(fail as i32));
    f.instruction(&Instruction::Call(import::FAIL));
    // fast：kind==0（纯错误）→ 内层（depth>1）补 (procedure …) 帧
    // 后以 (哨兵, ERROR) 多值返回；外层的帧由 call_proc 边界补。
    if e.fast.is_some() {
        f.instruction(&Instruction::LocalTee(e.kind));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::GlobalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32GtU);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::GlobalGet(3)); // cur_proc（wrapper 设置）
        f.instruction(&Instruction::Call(import::PROC_EXIT_FRAME));
        f.instruction(&Instruction::End);
        ret_fast_sentinel(f, code::ERROR);
        f.instruction(&Instruction::End);
        return;
    }
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
/// 范围检查（结果须落 [-2^62, 2^62)，即 >>62 ∈ {0,-1}）；不满足 →
/// br 0（慢路径）。免临时寄存器（重算两次 shift——不占 scratch/
/// epoch_home，避免与持有中的计算结果/epoch 快照冲突）。
fn range_guard_br_slow(f: &mut Function, reg: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(62));
    f.instruction(&Instruction::I64ShrS);
    f.instruction(&Instruction::I64Eqz);
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(62));
    f.instruction(&Instruction::I64ShrS);
    f.instruction(&Instruction::I64Const(-1));
    f.instruction(&Instruction::I64Eq);
    f.instruction(&Instruction::I32Or);
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::BrIf(0));
}

fn imm_guard_br_slow(f: &mut Function, reg: u32) {
    f.instruction(&Instruction::LocalGet(reg));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64And);
    f.instruction(&Instruction::I64Eqz);
    // 非立即数 → br 0：退出内层 slow/host 块，落到慢路径（br 1 会
    // 跳过慢代码直达 done——M1' 遗留极性反转，长期被 Call-in-Sub 拒绝
    // 造成的回落掩盖）。
    f.instruction(&Instruction::BrIf(0));
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
            // fast：参数从 per-depth 参数区直读（PARAMS + depth*STRIDE
            // + slot*8——proc_main wrapper 拷贝；递归各层切片互不覆盖）。
            if let Some(fc) = &e.fast {
                if let Some(slot) = fc.param_slot.get(&var) {
                    f.instruction(&Instruction::I32Const(cache::PARAMS as i32));
                    f.instruction(&Instruction::GlobalGet(5));
                    f.instruction(&Instruction::I32Const(cache::PARAM_STRIDE as i32));
                    f.instruction(&Instruction::I32Mul);
                    f.instruction(&Instruction::I32Add);
                    f.instruction(&Instruction::I64Load(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    f.instruction(&Instruction::LocalSet(dst.0));
                    return;
                }
            }
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
        Instr::IncrVar { dst, var, amount, fail, site } => {
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
            range_guard_br_slow(f, dst.0);
            retag_in_place(f, dst.0);
            st64_local(f, vals, dst.0);
            st8(f, state, 2);
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // host:
            // 降级走站点原文 eval 路由（rtcl.call 全语义）：incr_var
            // 公共入口的错误文本与 cmd_incr（tclsh 对齐面）不同
            // （fuzz f004106："expected integer, got yes" vs
            // "expected integer but got \"yes\""）。**先 flush**：
            // 缓存里的旧值（如 "on"）必须写回 interp——否则 incr 在
            // interp 上把不存在/过期的变量当 0 创建（f004881/f002484）。
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(*site as i32));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Call(import::CALL));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
            f.instruction(&Instruction::End); // done
        }
        Instr::CallSite { dst, cmd_id, argc, fail, site, srcs, cont_back, brk_depth, cont_depth, esc, esc_depth, .. } => {
            emit_flush(e, f, *fail);
            // ── fast-tier 直呼：epoch 门 + 槽缓存 → call_indirect；
            // 未命中 resolve_proc 冷融合（解析+代打+回填缓存）。
            // 统一 rc 分派：OK/RETURN=值；ERROR→fail_path（帧+哨兵）；
            // BREAK/CONTINUE/STALE/EXIT 经多值返回向上传播（零宿主）。
            if let Some(fc) = &e.fast {
                for (i, r) in srcs.iter().enumerate() {
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalGet(r.0));
                    f.instruction(&Instruction::I64Store(MemArg {
                        offset: (cache::SCRATCH + (i as u32) * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                }
                f.instruction(&Instruction::Block(BlockType::Empty)); // $done
                f.instruction(&Instruction::Block(BlockType::Empty)); // $fallback
                // epoch 失配（宿主 impure 路由会刷新共享 epoch）→ 回落。
                f.instruction(&Instruction::GlobalGet(1));
                f.instruction(&Instruction::I64Const(fc.epoch as i64));
                f.instruction(&Instruction::I64Ne);
                f.instruction(&Instruction::BrIf(0));
                // 槽缓存读：mem[site_cache_base + site*4]
                f.instruction(&Instruction::GlobalGet(4));
                f.instruction(&Instruction::I32Const((*site as i32) * 4));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                f.instruction(&Instruction::LocalTee(e.kind));
                f.instruction(&Instruction::I32Const(-1));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::BrIf(0));
                // call_indirect (slot, argc, SCRATCH, 表idx=slot)——
                // 表下标是**第 4 个**栈操作数（独立实验踩坑：漏压则
                // "expected i32 but nothing on stack"）。
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(*argc as i32));
                f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::CallIndirect { type_index: ty::PROC_MAIN as u32, table_index: 0 });
                // [i64 val, i32 rc] → kind=rc，dst=val
                f.instruction(&Instruction::LocalSet(e.kind));
                f.instruction(&Instruction::LocalSet(dst.0));
                // rc == OK(0) 或 RETURN(2) → done
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Eqz);
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(code::RETURN));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::I32Or);
                f.instruction(&Instruction::BrIf(1));
                // rc == ERROR(1) → fail_path（帧 + (哨兵, ERROR)）
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(code::ERROR));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                fail_path(e, f, *fail);
                f.instruction(&Instruction::End);
                // 其余 rc（BREAK/CONTINUE/STALE/EXIT）传播（depth-- 由
                // wrapper 尾部统一——impl 侧曾双递减致深度转负、参数
                // 切片地址变负读到常量区句柄串，fib(3) 串 operand/
                // fib(5+) OOB trap 双症状的根因）
                f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::Return);
                f.instruction(&Instruction::End); // $fallback:
                // resolve_proc(site, argc, SCRATCH) → i64（冷融合）
                f.instruction(&Instruction::I32Const(*site as i32));
                f.instruction(&Instruction::I32Const(*argc as i32));
                f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
                f.instruction(&Instruction::Call(import::RESOLVE_PROC));
                f.instruction(&Instruction::LocalSet(dst.0));
                // 哨兵分支内：fail_path（kind 0 → 帧+(哨兵,ERROR) 返回）
                // + kind 臂（1-4/6 → (哨兵, 映射 rc) 返回）。**必须在 If
                // 内**——外面的 kind 局部是未初始化垃圾（fib(2) 实证：
                // 随机命中 CONTINUE 码泄漏到 proc 边界）。
                f.instruction(&Instruction::LocalGet(dst.0));
                f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
                f.instruction(&Instruction::I64Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                fail_path(e, f, *fail);
                for (k, rc) in [
                    (1i32, code::BREAK),
                    (2, code::CONTINUE),
                    (3, code::BREAK),
                    (4, code::CONTINUE),
                    (6, code::STALE),
                ] {
                    f.instruction(&Instruction::LocalGet(e.kind));
                    f.instruction(&Instruction::I32Const(k));
                    f.instruction(&Instruction::I32Eq);
                    f.instruction(&Instruction::If(BlockType::Empty));
                    ret_fast_sentinel(f, rc);
                    f.instruction(&Instruction::End);
                }
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::End); // $done
                return;
            }
            // M2 全案直连：已求值实参寄存器写暂存区（SCRATCH 协议）→
            // rtcl.call_argv(base, argc, site) 直传 dispatch_values，
            // 免站点原文 eval 全链。RTCL_JIT_SITE_EVAL=1（启动读一次）
            // 回退原路由（逐 shape 对照用）。
            if site_eval_fallback() {
                f.instruction(&Instruction::I32Const(*cmd_id as i32));
                f.instruction(&Instruction::I32Const(*site as i32));
                f.instruction(&Instruction::I32Const(*argc as i32));
                f.instruction(&Instruction::Call(import::CALL));
            } else {
                for (i, r) in srcs.iter().enumerate() {
                    f.instruction(&Instruction::I32Const(0));
                    f.instruction(&Instruction::LocalGet(r.0));
                    f.instruction(&Instruction::I64Store(MemArg {
                        offset: (cache::SCRATCH + (i as u32) * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                }
                f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
                f.instruction(&Instruction::I32Const(*argc as i32));
                f.instruction(&Instruction::I32Const(*site as i32));
                f.instruction(&Instruction::Call(import::CALL_ARGV));
            }
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
            // ④ kind 6 = Tier1 中途 bump：整单元 return STALE，驱动侧
            // 从 bump 命令的下一条顶层命令树走查重分派。
            f.instruction(&Instruction::LocalGet(e.kind));
            f.instruction(&Instruction::I32Const(6));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(code::STALE));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // kind 5 = Next 区域 continue 逃逸：跳第一个 body 区祖先
            // 循环的 cont（深度已解析）；无祖先/解析失败 → 单元返回
            // CONTINUE（proc 边界按 loop_signal 语义转错）。
            if esc.is_some() {
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(5));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                match esc_depth {
                    Some(d) => {
                        f.instruction(&Instruction::Br(d + 2));
                    }
                    None => {
                        f.instruction(&Instruction::I32Const(code::CONTINUE));
                        f.instruction(&Instruction::Return);
                    }
                }
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
        Instr::Concat { dst, srcs, fail: _ } => {
            // 暂存区写入 + 宿主拼接（vm_exec Concat：as_str 连接，不失败）。
            for (i, r) in srcs.iter().enumerate() {
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalGet(r.0));
                f.instruction(&Instruction::I64Store(MemArg {
                    offset: (cache::SCRATCH + (i as u32) * 8) as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
            f.instruction(&Instruction::I32Const(srcs.len() as i32));
            f.instruction(&Instruction::Call(import::CONCAT));
            f.instruction(&Instruction::LocalSet(dst.0));
        }
        Instr::ForeachStart { info, ngroups: _, counter, srcs, fail } => {
            // 数据值写暂存区（升序 = 组序；宿主逆序弹组对齐 vm_exec）。
            for (i, r) in srcs.iter().enumerate() {
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalGet(r.0));
                f.instruction(&Instruction::I64Store(MemArg {
                    offset: (cache::SCRATCH + (i as u32) * 8) as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(*info as i32));
            f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
            f.instruction(&Instruction::I32Const(srcs.len() as i32));
            f.instruction(&Instruction::Call(import::FOREACH_START));
            f.instruction(&Instruction::LocalSet(e.scratch));
            check_sentinel(e, f, e.scratch, *fail);
            // iters = untag(result); i = 0。
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalSet(e.foreach_base + counter * 2 + 1));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(e.foreach_base + counter * 2));
        }
        Instr::ForeachEntry { counter, b } => {
            let li = e.foreach_base + counter * 2;
            let lt = li + 1;
            f.instruction(&Instruction::LocalGet(li));
            f.instruction(&Instruction::LocalGet(lt));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(e.b(b.0)));
        }
        Instr::ForeachStep { counter, b } => {
            // i += 1；bb = (i < iters)。
            let li = e.foreach_base + counter * 2;
            let lt = li + 1;
            f.instruction(&Instruction::LocalGet(li));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(li));
            f.instruction(&Instruction::LocalGet(li));
            f.instruction(&Instruction::LocalGet(lt));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(e.b(b.0)));
        }
        Instr::ForeachBind { info, counter, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(*info as i32));
            f.instruction(&Instruction::LocalGet(e.foreach_base + counter * 2));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::Call(import::FOREACH_BIND));
            f.instruction(&Instruction::LocalSet(e.scratch));
            check_sentinel(e, f, e.scratch, *fail);
        }
        Instr::ForeachCollect { src } => {
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::FOREACH_COLLECT));
        }
        Instr::ForeachEnd { dst } => {
            f.instruction(&Instruction::Call(import::FOREACH_END));
            f.instruction(&Instruction::LocalSet(dst.0));
        }
        Instr::EvalScript { dst, src, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::EVAL_SCRIPT));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::EvalExpr { dst, src, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::EVAL_EXPR));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::LappendVar { dst, src, var, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(e.name_handle(*var)));
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::LAPPEND_VAR));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::SetSite(k) => {
            f.instruction(&Instruction::I32Const(*k as i32));
            f.instruction(&Instruction::GlobalSet(e.site_global()));
        }
        Instr::SetResult { src } => {
            if let Some(fc) = &e.fast {
                fc.res_reg.set(Some(src.0));
            } else {
                f.instruction(&Instruction::LocalGet(src.0));
                f.instruction(&Instruction::Call(import::SET_RESULT));
            }
        }
        Instr::ReturnCode { code } => {
            if let Some(fc) = &e.fast {
                // OK/RETURN：结果寄存器直读（消灭 set_result 宿主调用）；
                // 其余码带值传播（外层驱动解码；depth-- 由 wrapper 统一）。
                match *code {
                    code::OK | code::RETURN => {
                        if let Some(r) = fc.res_reg.get() {
                            f.instruction(&Instruction::LocalGet(r));
                        } else {
                            f.instruction(&Instruction::I64Const(abi::tag_imm(0)));
                        }
                    }
                    _ => {
                        f.instruction(&Instruction::I64Const(abi::ERR_SENTINEL));
                    }
                }
                f.instruction(&Instruction::I32Const(*code));
                f.instruction(&Instruction::Return);
            } else {
                f.instruction(&Instruction::I32Const(*code));
                f.instruction(&Instruction::Return);
            }
        }
        Instr::Fail { fail } => {
            f.instruction(&Instruction::I32Const(*fail as i32));
            f.instruction(&Instruction::Call(import::FAIL));
            f.instruction(&Instruction::Drop);
            if e.fast.is_some() {
                ret_fast_sentinel(f, code::ERROR);
            } else {
                f.instruction(&Instruction::I32Const(code::ERROR));
                f.instruction(&Instruction::Return);
            }
        }
        Instr::Exit { code } => {
            f.instruction(&Instruction::I32Const(*code));
            f.instruction(&Instruction::Call(import::EXIT));
            if e.fast.is_some() {
                ret_fast_sentinel(f, code::EXIT);
            } else {
                f.instruction(&Instruction::I32Const(code::EXIT));
                f.instruction(&Instruction::Return);
            }
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
    emit_flush(e, f, fail);
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
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(e.scratch));
            range_guard_br_slow(f, e.scratch);
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
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
            f.instruction(&Instruction::I64Const(-1));
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Xor);
            f.instruction(&Instruction::LocalSet(e.scratch));
            range_guard_br_slow(f, e.scratch);
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
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
            // 先算进 scratch（dst 与 a 别名——范围检查失败时慢路径还要
            // 读原始 a/b，不能先覆盖）。
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
            f.instruction(&Instruction::LocalSet(e.scratch));
            range_guard_br_slow(f, e.scratch);
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
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
            f.instruction(&Instruction::LocalSet(e.scratch));
            if kind == BinOp::Shl {
                range_guard_br_slow(f, e.scratch);
            }
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
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
