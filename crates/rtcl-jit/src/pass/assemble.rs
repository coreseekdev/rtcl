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
    /// C-float（§5）：f64 双暂存 local（操作数缓存），float lane on 时
    /// 算术/比较的守卫-内联路径用。恒为该函数 local 末组。
    float_lane: bool,
    fa: u32,
    fb: u32,
    /// M2 slot 单态缓存（classic tier）：per-var 的 wasm local 组——
    /// `ic_base+var` = 缓存的 tagged 值（i64），`iv_base+var` = 有效位
    /// （i32，1=有效）。有效不变式：ivalid=1 ⇒ caching==1 且缓存值 ==
    /// interp 变量值。维护：fill 仅在 caching==1 语境；caching=1→0 的
    /// 全部转移点（binary_op/call/call_argv/foreach×2/eval×2/lappend
    /// 宿主 import）之后整表失效；StoreVar/IncrVar 写同步刷新。
    ic_base: u32,
    iv_base: u32,
    /// fast-tier（proc_main 发射中）：Some(ctx) 时——LoadVar(参数)→
    /// local.get；SetResult 免宿主直记寄存器；fail/出口返回
    /// (i64 哨兵, i32 rc) 多值；cur_site global 下标 = 4（多导入
    /// cur_proc 一个）。
    fast: Option<FastCtx>,
}

struct FastCtx {
    /// var（l2.vars 下标）→ slot（per-depth 参数区偏移 = slot*8）；
    /// 两段布局：[0,nparams) 参数段（wrapper 预填非 0），[nparams,)
    /// 局部段（wrapper 清零 = 未写）。
    param_slot: std::collections::HashMap<u32, u32>,
    nparams: u32,
    /// 槽数（= nvars，≤ MAX_SLOTS_PER_PROC——末槽保留给脏位掩码）。
    nslots: u32,
    /// slot → name handle（lazy-flush 逆查；param_slot 的反转）。
    slot_names: Vec<i32>,
    /// 字节码 locals 槽 → fast 槽（foreach 循环变量定向 resync 用）。
    slot_of_local: std::collections::HashMap<u32, u32>,
    /// foreach info → [(fast_slot, name_h)]（循环变量定向 resync 表；
    /// 无 fast 槽的循环变量不在表内——其读取走宿主路由）。
    foreach_var_handles: std::collections::HashMap<u32, Vec<(u32, i32)>>,
    /// var_idx → 名字句柄（调用后槽重同步）。
    var_names: Vec<i32>,
    /// resync 临时 i64 local 下标（fast_impl 的 i64 组末尾追加位）。
    tmp64: u32,
    /// 槽切片基址缓存 i32 local（fast_impl 入口算一次）。
    slot_base: u32,
    /// 体含 StoreVar/IncrVar（槽写）才需要 lazy-flush 机器——纯读
    /// 形状（fib）掩码恒 0，wrapper 清零/出口 flush/CallSite flush
    /// 全免（每调用 ~12 指令协议开销）。
    has_slot_writes: bool,
    /// Cmp-BoolOfBool 融合表（Cmp dst reg -> 目标 BReg）：Cmp 结果仅被
    /// 紧随 BoolOfBool 消费（全单元 use-count==1）时，fast 臂 i32 比较
    /// 结果直写 b 寄存器、BoolOfBool 空转（循环条件省 ~7 指令/迭代）。
    fused_cmp: std::collections::HashMap<u32, u32>,
    /// 体含 CallSite 才需要调用后 resync（纯算术叶子无人可写本帧
    /// 槽——fib 内层叶路径免 per-var 宿主读，性能关键）。
    resync_on: bool,
    /// SetResult 记录的结果寄存器（ReturnCode 出口读取）。
    res_reg: std::cell::Cell<Option<u32>>,
    /// 编译期 tier1 epoch（CallSite 槽缓存失效基准）。
    epoch: u64,
}

impl Env<'_> {
    /// fast 模式下的 cur_site global 下标（4 个导入 global）。
    fn site_global(&self) -> u32 {
        // fast 模块导入 7 个 global（0-6），内部 cur_site = 7
        //（fast 6 是 dirty——resync 门专用，SetSite 不得碰）。
        if self.fast.is_some() { 7 } else { 3 }
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
    let nv = l2.max_depth + 1;
    if std::env::var_os("JIT_L3").is_some() {
        eprintln!("=== L3 (sites={}) ===\n{}", code.sites.len(), l3);
    }
    let bregs = count_bregs(l3);
    let nf = count_foreach(l3);
    let foreach_base = nv + 3;
    if std::env::var_os("JIT_FT").is_some() { eprintln!("[gate] {:?}", fast_gate(l2,l3,code)); }
    let nparams = fast_gate(l2, l3, code);
    let nvars = l2.vars.len() as u32;
    let env = Env {
        l2,
        foreach_base,
        scratch: nv + 1,
        epoch_home: nv + 2,
        // i64 组加了 icache（nvars 枚）——i32 侧全部绝对基址随之平移
        // （wasm local 索引跨组连续；bool/kind/ivalid 不得撞 icache 区）。
        bool_base: foreach_base + 2 * nf + nvars,
        kind: foreach_base + 2 * nf + nvars + bregs,
        // 缓存 local 组：icache 接在 i64 组末，ivalid 接在 i32 组末。
        ic_base: nv + 3 + 2 * nf,
        iv_base: nv + 3 + 2 * nf + nvars + bregs + 1,
        // C-float：f64 组恒在该函数 local 末尾（2 枚：操作数 a/b 暂存）。
        float_lane: cfg.float_lane,
        fa: nv + 3 + 2 * nf + nvars + bregs + 1 + nvars,
        fb: nv + 3 + 2 * nf + nvars + bregs + 1 + nvars + 1,
        fast: None,
    };

    let mut body = Function::new(vec![
        (nv + 3 + 2 * nf + nvars, I64), // 值栈 + 出口 + SCRATCH + EPOCH + foreach 对 + icache
        (bregs + 1 + nvars, I32),       // 布尔寄存器 + kind + ivalid
        (2, ValType::F64),              // C-float 暂存（fa/fb）
    ]);

    // ── 序言：cache 区清零 + caching=1（池重置的 wasm 侧承担）──────
    // 池命中的宿主重置此前每调用写 512+vars 字节 + 1 次 global.set
    //（~120ns）；搬进序言后每 var 2 条 store（~2ns）+ 1 条 global.set，
    // 且宿主 pool_reset 瘦身为纯 reset_for_reuse。语义：清零发生在
    // 任何 LoadVar 之前——cache 契约「caching=1 ⇒ VALUES 槽新鲜」在
    // 运行起点成立；运行中的 impure 路由仍以 caching=0 失效整窗。
    // 变量数 = l2.vars.len()（名字键变量表）——nv 是值栈寄存器数，
    // 两者无关；少清会让池复用读到陈旧缓存（namespace-old-7.10 实证）。
    // （nvars 已在 env 构造前计算。）
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
    imports.import("rtcl", "sync_var", EntityType::Function(ty::I32_TO_I64));
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
        imports.import("rtcl", "dirty", EntityType::Global(GlobalType {
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
        let mut nvars2 = 0usize;
        for (vi, v) in l2.vars.iter().enumerate() {
            if let Some(slot) = v.const_idx.checked_sub(consts_len2) {
                nvars2 = nvars2.max(slot as usize + 1);
                param_slot.insert(vi as u32, slot);
            }
        }
        let nvars = nvars2;
        // M2 缓存组（fast 侧预留；classic 同字段）：icache 追加在 i64
        // 组末（TMP64 之后）——i32 侧全部绝对基址随之平移（同 run 的
        // 撞位教训）。
        let fb_ic = l2.vars.len() as u32;
        // fast_impl 的 i64 组比 run 多一枚 TMP64（resync 临时，落位
        // nv+3+2nf）——fast 语境的全部寄存器基址随之 +1（WAT 反汇编
        // 实证：Env 沿用 run 基址时首个 i32 寄存器与 TMP64 撞位，
        // i64/i32 型错配 = 模块校验失败）。
        let fast_foreach_base = nv + 5 + 2 * nf + fb_ic;
        // 槽切片基址缓存 i32 local（入口算一次 = PARAMS + depth*STRIDE，
        // fast_impl 内恒定）：所有槽寻址 5 指令 → 1 local.get（fl 循环
        // 8 个寻址点/迭代，~35 指令/迭代）。
        let slot_base_local = nv + 4 + 2 * nf + fb_ic;
        let vnh = {
            let mut v = vec![0i32; l2.vars.len()];
            for (vi, vv) in l2.vars.iter().enumerate() {
                v[vi] = crate::abi::const_handle(vv.const_idx) as i32;
            }
            v
        };
        let slot_names = {
            let mut sn = vec![0i32; nvars];
            for (vi, slot) in &param_slot {
                sn[*slot as usize] = vnh[*vi as usize];
            }
            sn
        };
        // 字节码 locals 槽 → fast 槽（foreach 循环变量定向 resync 用）。
        let slot_of_local = {
            let mut sl = std::collections::HashMap::new();
            for (vi, v) in l2.vars.iter().enumerate() {
                if let Some(fs) = param_slot.get(&(vi as u32)) {
                    if let Some(ls) = v.const_idx.checked_sub(consts_len2) {
                        sl.insert(ls, *fs);
                    }
                }
            }
            sl
        };
        // foreach info → [(fast_slot, name_h)]（循环变量定向 resync 表；
        // 无 fast 槽的循环变量不在表内——其读取走宿主路由）。
        let foreach_var_handles = {
            let mut fh: std::collections::HashMap<u32, Vec<(u32, i32)>> =
                std::collections::HashMap::new();
            for ii in 0..code.foreach_info_len() {
                if let Some(fi) = code.foreach_info(ii) {
                    let mut list = Vec::new();
                    for g in &fi.groups {
                        for t in g {
                            if let Some(ls) = t.slot {
                                if let Some(fs) = slot_of_local.get(&(ls as u32)) {
                                    list.push((*fs, crate::abi::const_handle(t.name_idx as u32) as i32));
                                }
                            }
                        }
                    }
                    fh.insert(ii, list);
                }
            }
            fh
        };
        let has_slot_writes = {
            let mut w = false;
            walk(l3, &mut |n| {
                if matches!(
                    n,
                    L3::Instr(Instr::StoreVar { .. }) | L3::Instr(Instr::IncrVar { .. })
                ) {
                    w = true;
                }
            });
            w
        };
        let fenv = Env {
            l2,
            // foreach i/iters 是 i64 对：基址 = nv+3（与经典同——fast
            // i64 组 nv+4+2nf 已含对区，TMP64=nv+3+2nf 恰在其后）。
            // fast_foreach_base 是 i32 侧 bool/kind 基址公式（沿用），
            // 不是 i64 基址——原误用会让 nf>0 时 foreach 对写进 i32
            // 组（验证失败/类型错配）。
            foreach_base: nv + 3,
            scratch: nv + 1,
            epoch_home: nv + 2,
            bool_base: fast_foreach_base + 2 * nf,
            kind: fast_foreach_base + 2 * nf + bregs,
            // fast 侧缓存组（件3 预留；classic 语义同字段）——icache
            // 接在 TMP64（i64 组末）之后；ivalid 接在 kind（i32 组末）
            // 之后。
            ic_base: nv + 4 + 2 * nf,
            iv_base: fast_foreach_base + 2 * nf + bregs + 1,
            // C-float：fast_impl 的 f64 组恒在其 local 末尾（与 classic
            // 同型——i64 组 + i32 组之后追加 2 枚）。
            float_lane: cfg.float_lane,
            fa: nv + 4 + 2 * nf + fb_ic + bregs + 2 + 2 * nf + fb_ic,
            fb: nv + 4 + 2 * nf + fb_ic + bregs + 2 + 2 * nf + fb_ic + 1,
            fast: Some(FastCtx {
                param_slot,
                nparams: np as u32,
                nslots: nvars as u32,
                slot_names: slot_names.clone(),
                slot_of_local,
                foreach_var_handles,
                var_names: vnh,
                tmp64: nv + 3 + 2 * nf,
                slot_base: slot_base_local,
                has_slot_writes,
                fused_cmp: scan_fused_cmps(l3),
                resync_on: {
                    let mut hc = false;
                    walk(l3, &mut |n| {
                        if matches!(n, L3::Instr(Instr::CallSite { .. })) {
                            hc = true;
                        }
                    });
                    hc
                },
                res_reg: std::cell::Cell::new(None),
                epoch: code.epoch,
            }),
        };
        // fast_impl：局部 = run 布局（无参数移位）+ slot_base i32
        // + M2 缓存组（icache i64 × nvars、ivalid i32 × nvars）。
        let mut fb = Function::new(vec![
            (nv + 4 + 2 * nf + fb_ic, I64), // +TMP64（resync 临时）+ icache
            (bregs + 2 + 2 * nf + fb_ic, I32), // +slot_base（i32 组首位）+ ivalid
            (2, ValType::F64),              // C-float 暂存（fa/fb）
        ]);
        // 入口：slot_base = PARAMS + fast_depth * PARAM_STRIDE
        fb.instruction(&Instruction::I32Const(cache::PARAMS as i32));
        fb.instruction(&Instruction::GlobalGet(5));
        fb.instruction(&Instruction::I32Const(cache::PARAM_STRIDE as i32));
        fb.instruction(&Instruction::I32Mul);
        fb.instruction(&Instruction::I32Add);
        fb.instruction(&Instruction::LocalSet(slot_base_local));
        emit_node(l3, &fenv, &mut fb);
        fb.instruction(&Instruction::End);
        fast_impl = Some(fb);

        // proc_main wrapper：locals = 2 i32（saved_cur, param_base）
        // + 出口暂存（val i64 / rc i32）——结果对不能垫在 flush 栈下：
        // else-less if 的 then 分支不可弹出帧基以下操作数（join 高度
        // 不变式），必须先落 local。
        let mut fw = Function::new(vec![(2, I32), (1, I64), (1, I32)]);
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
        // 局部段清零 [np, nvars)：上一同深度调用残留即脏帧。
        for i in np as u32..nvars as u32 {
            fw.instruction(&Instruction::LocalGet(4));
            fw.instruction(&Instruction::I32Const((i * 8) as i32));
            fw.instruction(&Instruction::I32Add);
            fw.instruction(&Instruction::I64Const(0));
            fw.instruction(&Instruction::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
        }
        // 脏位掩码清零（切片末字）——同上，防残留（纯读单元免）。
        if has_slot_writes {
            fw.instruction(&Instruction::LocalGet(4));
            fw.instruction(&Instruction::I64Const(0));
            fw.instruction(&Instruction::I64Store(MemArg {
                offset: cache::SLOT_MASK_OFF as u64,
                align: 3,
                memory_index: 0,
            }));
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
        // 尾调 fast_impl → [i64, i32] → 先落 local（val=5, rc=6）：
        // 后续 flush 的错误分支要发 (哨兵, ERROR)，若结果对垫在栈下，
        // else-less if 的 then 分支弹不到帧基以下操作数（join 高度
        // 不变式），必须经 local 中转。
        fw.instruction(&Instruction::Call(import::FAST_IMPL));
        fw.instruction(&Instruction::LocalSet(6));
        fw.instruction(&Instruction::LocalSet(5));
        // lazy-flush 出口落盘：调用方 resync（dirty 门）从 interp 帧拉
        // 真值——本帧脏槽必须先落（uv2：p2 经 upvar 写 p1 的槽，写停
        // 在 p2 槽掩码里，不落则 p1 拉回旧值）。纯读单元免；掩码空
        // 早退——脏形状每调用只付一次 load+test。
        if has_slot_writes {
        fw.instruction(&Instruction::Block(BlockType::Empty)); // $noflush
        fw.instruction(&Instruction::LocalGet(4));
        fw.instruction(&Instruction::I64Load(MemArg {
            offset: cache::SLOT_MASK_OFF as u64,
            align: 3,
            memory_index: 0,
        }));
        fw.instruction(&Instruction::I64Eqz);
        fw.instruction(&Instruction::BrIf(0));
        for slot in 0..nvars as u32 {
            fw.instruction(&Instruction::Block(BlockType::Empty));
            fw.instruction(&Instruction::LocalGet(4));
            fw.instruction(&Instruction::I64Load(MemArg {
                offset: cache::SLOT_MASK_OFF as u64,
                align: 3,
                memory_index: 0,
            }));
            fw.instruction(&Instruction::I64Const(slot as i64));
            fw.instruction(&Instruction::I64ShrU);
            fw.instruction(&Instruction::I64Const(1));
            fw.instruction(&Instruction::I64And);
            fw.instruction(&Instruction::I64Eqz);
            fw.instruction(&Instruction::BrIf(0));
            fw.instruction(&Instruction::I32Const(slot_names[slot as usize]));
            fw.instruction(&Instruction::LocalGet(4));
            fw.instruction(&Instruction::I64Load(MemArg {
                offset: (slot * 8) as u64,
                align: 3,
                memory_index: 0,
            }));
            fw.instruction(&Instruction::Call(import::SET_VAR));
            // 失败（pending 已由宿主置）：恢复栈，(哨兵, ERROR)
            fw.instruction(&Instruction::I32Const(0));
            fw.instruction(&Instruction::I32Ne);
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
        }
        fw.instruction(&Instruction::LocalGet(4));
        fw.instruction(&Instruction::I64Const(0));
        fw.instruction(&Instruction::I64Store(MemArg {
            offset: cache::SLOT_MASK_OFF as u64,
            align: 3,
            memory_index: 0,
        }));
        fw.instruction(&Instruction::End); // $noflush
        }
        // 恢复栈；透传返回
        fw.instruction(&Instruction::GlobalGet(5));
        fw.instruction(&Instruction::I32Const(1));
        fw.instruction(&Instruction::I32Sub);
        fw.instruction(&Instruction::GlobalSet(5));
        fw.instruction(&Instruction::LocalGet(3));
        fw.instruction(&Instruction::GlobalSet(3));
        fw.instruction(&Instruction::LocalGet(5));
        fw.instruction(&Instruction::LocalGet(6));
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
    // ABI 版本 export（docs/ABI-OPT-NOTES.md §3a）：局部段末追加，
    // 不触碰 import::RUN/FAST_IMPL 索引；复用 ty::RUN 签名 () -> i32。
    // 索引：常规 = 28（RUN 27 之后）；fast = 30（RUN/fast_impl/
    // proc_main 之后）。
    let ver_idx = import::COUNT + 1 + if has_fast { 2 } else { 0 };
    funcs.function(ty::RUN);

    let mut globals = GlobalSection::new();
    // 导入 global 数：常规 3（fuel/epoch/caching）→ cur_site=3；fast 7
    // （+cur_proc/site_cache_base/fast_depth/dirty）→ cur_site=7。此前
    // fast 侧写/export 都指 6——dirty 引入后没同步，SetSite 一直把站点
    // 号写进 dirty（假触发 resync 门）。
    globals.global(
        GlobalType { val_type: I32, mutable: true, shared: false },
        &wasm_encoder::ConstExpr::i32_const(-1),
    );

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, import::RUN);
    exports.export("rtcl_abi_version", ExportKind::Func, ver_idx);
    exports.export("cur_site", ExportKind::Global, if has_fast { 7 } else { 3 });
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
    // 版本函数体（与函数段追加序一致——局部段末）：const 返回
    // ABI_VERSION，无参无 local。
    let mut ver_body = Function::new(vec![]);
    ver_body.instruction(&Instruction::I32Const(abi::ABI_VERSION));
    ver_body.instruction(&Instruction::End);
    code_section.function(&ver_body);

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
    // 顶层 slot 单元（C4 重开）：foreach/lmap 已在编译侧排除
    //（eval.rs has_foreach → 非 slot 编译），分派中 lmap/continue 的
    // 体作用域分歧源已消除。既有 flush/resync 纪律覆盖全局别名。
    let np = match code.params_aligned {
        Some(np) => np,
        None => {
            if std::env::var_os("JIT_FT").is_some() {
                eprintln!("[gate] reject: params_aligned=None");
            }
            return None;
        }
    };
    // VarSlot.const_idx 是**池相对**索引（name_handle 才加 CONST_BASE）——
    // local 伪常量段 = [consts_len, consts_len+locals)。此前误减
    // l2.const_base（=3）致 fib 的参数 n 被判非常量而拒绝。
    let consts_len = code.constants().len() as u32;
    let mut nvars = 0usize;
    for v in &l2.vars {
        let slot = match v.const_idx.checked_sub(consts_len) {
            Some(s) => s as usize,
            None => {
                if std::env::var_os("JIT_FT").is_some() {
                    eprintln!("[gate] reject: var const_idx below consts_len (非局部变量)");
                }
                return None;
            }
        };
        nvars = nvars.max(slot + 1);
    }
    if nvars > crate::abi::cache::MAX_SLOTS_PER_PROC {
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[gate] reject: nvars={nvars} > cap");
        }
        return None;
    }
    // site-cache 段容量（SITE_STRIDE/4 = 256 槽）：超长 proc 不 fast 化。
    if code.sites.len() > (crate::abi::cache::SITE_STRIDE / 4) as usize {
        return None;
    }
    // v3: loops allowed
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
            // v4：foreach 语义全绿但 perf 回退 gate 收回——fast 臂每
            // 迭代 flush+bind+sync 三次宿主调用 ~200ns，interp 内联
            // foreach 直写槽 ~16ns/迭代（fe_bench 20k：fast 4.1ms vs
            // interp 0.32ms = 12×劣化；回落 interp=持平）。RTCL_JIT_
            // FE_FAST=1 重新放行（实验档，正确性已由本轮全绿背书）。
            Instr::ForeachStart { .. }
            | Instr::ForeachBind { .. }
            | Instr::ForeachStep { .. }
            | Instr::ForeachEnd { .. }
            | Instr::ForeachCollect { .. }
                if std::env::var("RTCL_JIT_FE_FAST").as_deref() != Ok("1") =>
            {
                Some("foreach")
            }
            // v3.1：BackedgeFuel 放行——emit_fuel 只用 fuel global(0) +
            // REFUEL import + check_sentinel，fast 模块全部可用
            // （fast refuel 闭包 = charge_step 循环，与经典同语义）。
            _ => None,
        };
        if b.is_some() && bad.is_none() {
            bad = b;
        }
    });
    if bad.is_some() {
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[gate] reject: {}", bad.unwrap());
        }
        return None;
    }
    // v5：循环体内 CallSite 拒绝——每迭代 CallSite 协议（flush + 宿主
    // 分派 + dispatch 后无条件 dirty→resync）~300-500ns，interp 的
    // DynCall/内联分派 ~190ns：dispatch 密集循环 fast 净亏 2.7×
    //（tc lindex 循环实测）。回落 exec_bytecode = interp 平价。
    // proc 互调循环（call_indirect 无 resync）理论可赢——无实测证据
    // 前一并拒绝，出现反例再细分。
    // aggressive（wasm32/RTCL_JIT_AGGRESSIVE=1）：解释器基线 wasm 化
    // 后反转，平价形状按用户指令保留 JIT 路径，不拒绝。
    if !crate::aggressive_tier() && callsite_in_loop(l3) {
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[gate] reject: callsite-in-loop");
        }
        return None;
    }
    let _ = (&bad, has_loop);
    Some(np)
}

/// L3 循环（含循环内 If/Block 嵌套）体内是否有 CallSite。
pub(crate) fn callsite_in_loop(n: &L3) -> bool {
    match n {
        L3::Loop { body, .. } => {
            fn has_call(m: &L3) -> bool {
                match m {
                    L3::Instr(Instr::CallSite { .. }) => true,
                    L3::Seq(items) => items.iter().any(has_call),
                    L3::If { then_, else_, .. } => has_call(then_) || has_call(else_),
                    L3::Block { body, .. } => has_call(body),
                    L3::Loop { .. } | L3::Br { .. } | L3::BrIf { .. } => false,
                    _ => false,
                }
            }
            has_call(body)
        }
        L3::Seq(items) => items.iter().any(callsite_in_loop),
        L3::If { then_, else_, .. } => callsite_in_loop(then_) || callsite_in_loop(else_),
        L3::Block { body, .. } => callsite_in_loop(body),
        _ => false,
    }
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

/// Reg 操作数枚举（uses/defs 分计——非 SSA 即放弃融合）。
fn reg_use_def(i: &Instr, r: u32, uses: &mut usize, defs: &mut usize) {
    match i {
        Instr::Const { dst, .. } | Instr::BoxFloat { dst, .. } | Instr::LoadVar { dst, .. }
        | Instr::IncrVar { dst, .. } | Instr::ForeachEnd { dst } => {
            if dst.0 == r { *defs += 1 }
        }
        Instr::Copy { dst, src }
        | Instr::EvalScript { dst, src, .. }
        | Instr::EvalExpr { dst, src, .. }
        | Instr::LappendVar { dst, src, .. } => {
            if dst.0 == r { *defs += 1 }
            if src.0 == r { *uses += 1 }
        }
        Instr::Concat { dst, srcs, .. } | Instr::CallSite { dst, srcs, .. } => {
            if dst.0 == r { *defs += 1 }
            for x in srcs {
                if x.0 == r { *uses += 1 }
            }
        }
        Instr::ForeachStart { srcs, .. } => {
            for x in srcs {
                if x.0 == r { *uses += 1 }
            }
        }
        Instr::ForeachCollect { src } | Instr::StoreVar { src, .. } | Instr::SetResult { src } => {
            if src.0 == r { *uses += 1 }
        }
        Instr::Arith { dst, a, b, .. } | Instr::Cmp { dst, a, b, .. } => {
            if dst.0 == r { *defs += 1 }
            if a.0 == r { *uses += 1 }
            if b.0 == r { *uses += 1 }
        }
        Instr::Unary { dst, a, .. } => {
            if dst.0 == r { *defs += 1 }
            if a.0 == r { *uses += 1 }
        }
        Instr::BoolTest { v, .. } | Instr::BoolOfBool { v, .. } => {
            if v.0 == r { *uses += 1 }
        }
        _ => {}
    }
}

/// Cmp-BoolOfBool 融合扫描：同层相邻、v==dst、全单元 defs==1 且
/// uses==1（唯一读者就是紧随的转换）。
fn scan_fused_cmps(l3: &L3) -> std::collections::HashMap<u32, u32> {
    let mut out = std::collections::HashMap::new();
    fn go(n: &L3, root: &L3, out: &mut std::collections::HashMap<u32, u32>) {
        match n {
            L3::Seq(items) => {
                for w in items.windows(2) {
                    if let (
                        L3::Instr(Instr::Cmp { dst, .. }),
                        L3::Instr(Instr::BoolOfBool { dst: breg, v }),
                    ) = (&w[0], &w[1])
                    {
                        if v.0 == dst.0 {
                            let (mut uses, mut defs) = (0usize, 0usize);
                            walk(root, &mut |x| {
                                if let L3::Instr(i) = x {
                                    reg_use_def(i, dst.0, &mut uses, &mut defs);
                                }
                            });
                            if uses == 1 && defs == 1 {
                                out.insert(dst.0, breg.0);
                            }
                        }
                    }
                }
                for it in items {
                    go(it, root, out);
                }
            }
            L3::If { then_, else_, .. } => {
                go(then_, root, out);
                go(else_, root, out);
            }
            L3::Loop { body, .. } | L3::Block { body, .. } => go(body, root, out),
            _ => {}
        }
    }
    go(l3, l3, &mut out);
    out
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
/// 「range 不 ok 则落宿主」的守卫片段。`host_br` = 溢出时 br 的深度：
/// classic 两层布局 [done, slow] 内 = 0（退出 slow → 宿主）；float-lane
/// 四层布局 [done, host, flt, intp] 内 = 1（退出 intp+flt → 宿主）。
/// **溢出必须落宿主**（wide/bignum 全语义）——落 float 路径会把溢出的
/// int 收缩成 f64（4611686018427387903+4611686018427387903 会错成
/// 9.22337203685478e18，tclsh 是精确 wide 9223372036854775806）。
/// 范围检查（结果须落 [-2^62, 2^62)，即 >>62 ∈ {0,-1}）。
/// 免临时寄存器（重算两次 shift——不占 scratch/epoch_home，避免与
/// 持有中的计算结果/epoch 快照冲突）。
fn range_guard_br(f: &mut Function, reg: u32, host_br: u32) {
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
    f.instruction(&Instruction::BrIf(host_br));
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

/// C-float 操作数取值（§5）：Float rep → unbox_float；imm int →
/// f64.convert_i64_s 提升（Tcl 混合运算语义：int+float=float，转换
/// 舍入与 tclsh 的 int→double 一致）；其余（串 shimmer/bignum/整值
/// 句柄）→ br 宿主兜底。调用方块布局 [done, host, flt]——本片段内
/// 深度：imm=0, op=1, flt=2, host=3, done=4（$imm 内）。
///
/// **br 目标语义（2026-11-18 修正）**：br N = 跳到第 N 层块的末尾之后
/// 继续执行。宿主代码在 flt 的 End 之后、host 的 End 之内——所以要
/// **进入宿主代码**必须 br 到 flt 的 label（blk 内 = 3，op 内 = 1）；
/// br 到 host 的 label（4）会连宿主代码一起跳过 → dst 不写 → 返回
/// 操作数 a 原值（b1-b9 实证：`$r+1.0` 产出 1.5；imm_guard_br_slow 的
/// 「br 0 落慢路径 / br 1 跳过慢代码」历史注释同构验证）。
///
/// `exact_only`（比较语境）：tclsh 对 int vs double 比较**精确裁决**
/// （`9007199254740993 > 9007199254740992.0` = 1——wide/double 走
/// TclCompareTwoNumbers 精确路径，不做 double 收缩）；而 f64 提升会
/// 把 >2^53 的 imm 收缩成邻值 → 恒等分歧。故比较语境只放行 |v|<2^53
/// 的 imm（IEEE 精确域），其余落宿主。算术语境不设限：tclsh 的
/// int→double 算术提升同样舍入（`+0.0` 实证），convert 舍入一致。
fn emit_float_operand(f: &mut Function, r: u32, dst_f: u32, exact_only: bool) {
    f.instruction(&Instruction::Block(BlockType::Empty)); // $op
    f.instruction(&Instruction::Block(BlockType::Empty)); // $imm
    // **imm 检查前置**（2026-11-18 t3b/t3e SEGFAULT 教训）：unbox_float
    // 的入参是 I32WrapI64——大 imm 的 tagged 值（如 2^54+3）截断成小
    // 偶数 → 被当句柄探进 arena → clone 越界值 → SIGSEGV（2^62-1 截断
    // 成 -1 探空侥幸存活）。imm 永非 Float rep，先分派：imm 直接走
    // 提升/精确守卫，只有真句柄（even，wrap 精确）才调 unbox。
    f.instruction(&Instruction::LocalGet(r));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64And);
    f.instruction(&Instruction::I64Eqz);
    f.instruction(&Instruction::BrIf(0)); // 非 imm → 退出 $imm → unbox 路径
    // imm 臂：
    if exact_only {
        // 精确域守卫：v = r>>1，|v| < 2^53 才提升（>>53 ∈ {0,-1}）；
        // 越界（如 2^53 邻域的 wide imm）→ host 精确比较（tclsh 对
        // wide/double 比较精确裁决，9007199254740993>9007199254740992.0
        // = 1 实证）。
        for (k, expect) in [(53i64, 0i64), (53, -1)] {
            f.instruction(&Instruction::LocalGet(r));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Const(k));
            f.instruction(&Instruction::I64ShrS);
            if expect == 0 {
                f.instruction(&Instruction::I64Eqz);
            } else {
                f.instruction(&Instruction::I64Const(expect));
                f.instruction(&Instruction::I64Eq);
            }
        }
        f.instruction(&Instruction::I32Or);
        f.instruction(&Instruction::I32Eqz);
        // $imm 内深度：imm=0, op=1, flt=2, host=3——br 2 = 退出 flt →
        // 宿主代码（br 3 会连宿主一起跳过）。
        f.instruction(&Instruction::BrIf(2)); // |v| ≥ 2^53 → 宿主精确裁决
    }
    // imm int 提升（f64.convert_i64_s 舍入与 tclsh int→double 一致）。
    f.instruction(&Instruction::LocalGet(r));
    f.instruction(&Instruction::I64Const(1));
    f.instruction(&Instruction::I64ShrS);
    f.instruction(&Instruction::F64ConvertI64S);
    f.instruction(&Instruction::LocalSet(dst_f));
    // 提升完成 → 跳过 unbox 与 NaN 检查（imm 产不出 NaN）：
    // br 2 = 退出 $imm+$op → float 运算段。
    f.instruction(&Instruction::Br(2));
    f.instruction(&Instruction::End); // $imm
    // unbox 路径（仅真句柄到达；even tagged → wrap 无损）：
    f.instruction(&Instruction::LocalGet(r));
    f.instruction(&Instruction::I32WrapI64);
    f.instruction(&Instruction::Call(import::UNBOX_FLOAT)); // [ok, v]
    f.instruction(&Instruction::LocalSet(dst_f)); // [ok]
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(BlockType::Empty)); // ok==0：非 Float rep
    // If 内深度：If=0, op=1, flt=2——br 2 = 退出 flt → 宿主代码。
    f.instruction(&Instruction::Br(2)); // 串 shimmer/整值句柄/bignum → 宿主
    f.instruction(&Instruction::End); // if
    // NaN 操作数守卫：interp 的 binary_op 对 NaN 值操作数报错
    //（"can't use non-numeric floating-point value as operand of"——
    // NaN 可经 Inf*0.0 存在但禁再入算术；NaN 值比较=0、Neg=NaN 不报
    // 错）。守卫统一落宿主：宿主就是 interp 本体，报错/结果自然对齐
    // （NaN 值在栈上存在但不再入 f64 内联）。$op 内深度：op=0, flt=1,
    // host=2, done=3——br 1 = 退出 flt → 宿主代码（br 2 会连宿主一起
    // 跳过）。
    f.instruction(&Instruction::LocalGet(dst_f));
    f.instruction(&Instruction::LocalGet(dst_f));
    f.instruction(&Instruction::F64Ne);
    f.instruction(&Instruction::BrIf(1)); // NaN → 宿主代码
    f.instruction(&Instruction::End); // $op
}

/// C-float 计算+边界：f64 运算 → box_float（走宿主 freelist，Inf/NaN
/// 是合法值——§5 不用 NaN 哨兵）；比较 f64.cc → tag_bool（IEEE754 =
/// C double 语义，NaN 比较恒假 = tclsh）。
///
/// `fused`（fast 融合表命中）：比较结果直写布尔寄存器 b(br) 而非 dst
/// ——与 int 快臂/慢臂同约定。raw cmp 的 i32（1=真）与慢臂补写的
/// `(tagged_dst != 1)`（tag(1)=3 → 1；tag(0)=1 → 0）真值一致，直写。
fn emit_float_op(kind: BinOp, dst: u32, fused: Option<u32>, e: &Env, f: &mut Function) {
    if kind == BinOp::Div {
        // 0.0/0.0 → interp 报 domain error（tclsh 拒造 NaN 除法结果，
        // expr_ops.rs：`b == 0.0 && a == 0.0` → domain_error）；f64 内联
        // 会静默产 NaN。±0 的 F64Eq 恒等 → 覆盖 -0.0 组合。float path
        // 裸区深度：If 内 If=0, flt=1, host=2——Br(1) = 退出 If+flt →
        // 宿主（其余 NaN 产生源已被操作数 NaN 守卫拦在宿主侧）。
        f.instruction(&Instruction::LocalGet(e.fa));
        f.instruction(&Instruction::F64Const(0.0.into()));
        f.instruction(&Instruction::F64Eq);
        f.instruction(&Instruction::LocalGet(e.fb));
        f.instruction(&Instruction::F64Const(0.0.into()));
        f.instruction(&Instruction::F64Eq);
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::Br(1)); // → host（domain error 全语义）
        f.instruction(&Instruction::End);
    }
    f.instruction(&Instruction::LocalGet(e.fa));
    f.instruction(&Instruction::LocalGet(e.fb));
    let op = match kind {
        BinOp::Add => &Instruction::F64Add,
        BinOp::Sub => &Instruction::F64Sub,
        BinOp::Mul => &Instruction::F64Mul,
        BinOp::Div => &Instruction::F64Div,
        BinOp::Eq => &Instruction::F64Eq,
        BinOp::Ne => &Instruction::F64Ne,
        BinOp::Lt => &Instruction::F64Lt,
        BinOp::Gt => &Instruction::F64Gt,
        BinOp::Le => &Instruction::F64Le,
        BinOp::Ge => &Instruction::F64Ge,
        _ => unreachable!("float op kind"),
    };
    f.instruction(op);
    if matches!(kind, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge) {
        if let Some(br) = fused {
            f.instruction(&Instruction::LocalSet(e.b(br))); // i32 在栈顶
        } else {
            tag_bool(f, dst); // i32 在栈顶
        }
    } else {
        f.instruction(&Instruction::Call(import::BOX_FLOAT));
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::LocalSet(dst));
    }
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

/// lazy-flush 协议：per-depth 切片基址（入口缓存的 slot_base local）。
fn fast_slot_base(f: &mut Function, sb: u32) {
    f.instruction(&Instruction::LocalGet(sb));
}

/// 槽脏位掩码置位（切片末字）：StoreVar/IncrVar fast 臂替代逐次
/// set_var 镜像——稳态循环零宿主调用，真值在宿主边界统一 flush。
/// 值经 tmp64 中转（store 栈序 = 地址在前）。
fn fast_mask_set(f: &mut Function, _tmp64: u32, sb: u32, slot: u32) {
    // 直排形式：i64.store 栈序 = 地址在前——先压地址，值在其上计算，
    // 免 tmp64 中转（-2 指令/次；稳态循环每迭代 2 次 mask 置位）。
    fast_slot_base(f, sb);
    fast_slot_base(f, sb);
    f.instruction(&Instruction::I64Load(MemArg {
        offset: cache::SLOT_MASK_OFF as u64,
        align: 3,
        memory_index: 0,
    }));
    f.instruction(&Instruction::I64Const(1i64 << slot.min(63)));
    f.instruction(&Instruction::I64Or);
    f.instruction(&Instruction::I64Store(MemArg {
        offset: cache::SLOT_MASK_OFF as u64,
        align: 3,
        memory_index: 0,
    }));
}

/// foreach 循环变量定向 resync：只把本 foreach 绑定的循环变量从
/// interp 拉回 fast 槽（bind 每迭代一次——全量 resync 的 nvars 次
/// sync_var 宿主调用让 fe_bench 20× 慢于 interp，定向后仅循环变量）。
/// 循环变量无 fast 槽（未进 param_slot）时跳过——其读取本就走宿主。
fn emit_resync_foreach(f: &mut Function, fc: &FastCtx, info: u32) {
    if let Some(list) = fc.foreach_var_handles.get(&info) {
        for (fs, name_h) in list {
            f.instruction(&Instruction::I32Const(*name_h));
            f.instruction(&Instruction::Call(import::SYNC_VAR));
            f.instruction(&Instruction::LocalSet(fc.tmp64));
            f.instruction(&Instruction::LocalGet(fc.slot_base));
            f.instruction(&Instruction::LocalGet(fc.tmp64));
            f.instruction(&Instruction::I64Store(MemArg {
                offset: (*fs * 8) as u64,
                align: 3,
                memory_index: 0,
            }));
        }
    }
}

/// 槽重同步（CallSite dirty 门体 + ForeachBind 臂共用）：每 var 一次
/// sync_var（名字读或 0）写回槽——宿主侧改写的帧变量拉回 fast 槽。
/// dirty global 的置位/清除由调用方负责（bind 恒同步、CallSite 门控）。
fn emit_resync_slots(e: &Env, f: &mut Function) {
    let fc = e.fast.as_ref().expect("emit_resync_slots outside fast ctx");
    if std::env::var_os("JIT_FT").is_some() {
        for (vi, v) in e.l2.vars.iter().enumerate() {
            let slot = fc.param_slot.get(&(vi as u32)).copied();
            eprintln!(
                "[resync-map] vi={vi} slot={:?} name_h={} const_idx={}",
                slot,
                fc.var_names.get(vi).copied().unwrap_or(-1),
                v.const_idx
            );
        }
    }
    for (vi, _) in e.l2.vars.iter().enumerate() {
        let slot = fc.param_slot.get(&(vi as u32)).copied().unwrap_or(0);
        let name_h = fc.var_names.get(vi).copied().unwrap_or(0);
        f.instruction(&Instruction::I32Const(name_h));
        f.instruction(&Instruction::Call(import::SYNC_VAR));
        f.instruction(&Instruction::LocalSet(fc.tmp64));
        f.instruction(&Instruction::LocalGet(fc.slot_base));
        f.instruction(&Instruction::LocalGet(fc.tmp64));
        f.instruction(&Instruction::I64Store(MemArg {
            offset: (slot * 8) as u64,
            align: 3,
            memory_index: 0,
        }));
    }
    // 同步后槽 = interp 真值，脏位作废。
    fast_slot_base(f, fc.slot_base);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I64Store(MemArg {
        offset: cache::SLOT_MASK_OFF as u64,
        align: 3,
        memory_index: 0,
    }));
}

/// 脏槽 flush：掩码逐位检查，命中即 set_var(名字, 槽值)（宿主帧对
/// 齐），随后清掩码。set_var 失败 → fail_path（pending 已由宿主置）。
/// 只在宿主可见边界发射：CallSite（命令分派/被调 upvar 读）与
/// proc_main 出口（调用方 resync 拉 interp 帧前必须落盘）。
fn emit_slot_flush(e: &Env, f: &mut Function, fail: u32) {
    let fc = e.fast.as_ref().expect("emit_slot_flush outside fast ctx");
    if !fc.has_slot_writes {
        return; // 纯读单元：掩码恒 0，零发射
    }
    // 掩码空早退：一次 load+test（纯读形状每次调用边界 ~4 指令）。
    f.instruction(&Instruction::Block(BlockType::Empty)); // $noflush
    fast_slot_base(f, fc.slot_base);
    f.instruction(&Instruction::I64Load(MemArg {
        offset: cache::SLOT_MASK_OFF as u64,
        align: 3,
        memory_index: 0,
    }));
    f.instruction(&Instruction::I64Eqz);
    f.instruction(&Instruction::BrIf(0));
    for slot in 0..fc.nslots {
        f.instruction(&Instruction::Block(BlockType::Empty)); // 本槽 done
        fast_slot_base(f, fc.slot_base);
        f.instruction(&Instruction::I64Load(MemArg {
            offset: cache::SLOT_MASK_OFF as u64,
            align: 3,
            memory_index: 0,
        }));
        f.instruction(&Instruction::I64Const(slot as i64));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::BrIf(0));
        f.instruction(&Instruction::I32Const(fc.slot_names[slot as usize]));
        fast_slot_base(f, fc.slot_base);
        f.instruction(&Instruction::I64Load(MemArg {
            offset: (slot * 8) as u64,
            align: 3,
            memory_index: 0,
        }));
        f.instruction(&Instruction::Call(import::SET_VAR));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        fail_path(e, f, fail);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End); // 本槽 done
    }
    fast_slot_base(f, fc.slot_base);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I64Store(MemArg {
        offset: cache::SLOT_MASK_OFF as u64,
        align: 3,
        memory_index: 0,
    }));
    f.instruction(&Instruction::End); // $noflush
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
                    // 三块模板（经典臂同构）：槽非零直读；零（局部段
                    // 未写）→ 宿主名字路由。
                    f.instruction(&Instruction::Block(BlockType::Empty)); // done
                    f.instruction(&Instruction::Block(BlockType::Empty)); // host
                    f.instruction(&Instruction::LocalGet(fc.slot_base));
                    f.instruction(&Instruction::I64Load(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    f.instruction(&Instruction::LocalTee(dst.0));
                    f.instruction(&Instruction::I64Eqz);
                    f.instruction(&Instruction::BrIf(0)); // 零 → host
                    f.instruction(&Instruction::Br(1)); // done
                    f.instruction(&Instruction::End); // host:
                    f.instruction(&Instruction::I32Const(e.name_handle(var)));
                    f.instruction(&Instruction::Call(import::GET_VAR));
                    f.instruction(&Instruction::LocalSet(dst.0));
                    check_sentinel(e, f, dst.0, *fail);
                    f.instruction(&Instruction::End); // done
                    return;
                }
            }
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            // M2：缓存有效臂先行（local 读 vs 既有 mem 读 + 零检查）。
            // 不变式：ivalid=1 ⇒ icache == interp 变量值（fill 只在
            // var-op 成功后；caching=1→0 转移点整表失效——见
            // invalidate_int_caches）。
            let ic = e.ic_base + var;
            let iv = e.iv_base + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // miss
            // 有效臂双门：caching==1（读/写 trace、别名读全在 caching=0
            // 世界走宿主——逐 op 可观察性不被缓存短路）且 iv==1。
            f.instruction(&Instruction::GlobalGet(2)); // caching
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0)); // → miss
            f.instruction(&Instruction::LocalGet(iv));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0)); // iv==0（无效）→ miss
            f.instruction(&Instruction::LocalGet(ic));
            f.instruction(&Instruction::LocalSet(dst.0));
            f.instruction(&Instruction::Br(1)); // → done
            f.instruction(&Instruction::End); // miss:
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
            // fill（caching==1 语境静态成立）。
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::LocalSet(ic));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(iv));
            // host block 与 miss block 是兄弟——done 从 host 内是深度 1。
            f.instruction(&Instruction::Br(1)); // → done
            f.instruction(&Instruction::End); // host:
            f.instruction(&Instruction::I32Const(e.name_handle(var)));
            f.instruction(&Instruction::Call(import::GET_VAR));
            f.instruction(&Instruction::LocalSet(dst.0));
            check_sentinel(e, f, dst.0, *fail);
            st64_local(f, vals, dst.0);
            st8(f, state, 1);
            // fill（host 臂：caching==0 的逐 op 读也刷新——icache 镜像
            // 「本 op 刚读到的值」，与 interp 恒一致）。
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::LocalSet(ic));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(iv));
            f.instruction(&Instruction::End); // done
        }
        Instr::StoreVar { src, var, keep: _, fail } => {
            let var = *var;
            if let Some(fc) = &e.fast {
                if let Some(slot) = fc.param_slot.get(&var) {
                    f.instruction(&Instruction::LocalGet(fc.slot_base));
                    f.instruction(&Instruction::LocalGet(src.0));
                    f.instruction(&Instruction::I64Store(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    // lazy-flush：置脏位（含参数段——参数也可被 set 改写，
                    // 此前仅局部段镜像、参数写对宿主帧不可见是既有缺口），
                    // 宿主边界统一 set_var。
                    fast_mask_set(&mut *f, fc.tmp64, fc.slot_base, *slot);
                    return;
                }
            }
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            let ic = e.ic_base + var;
            let iv = e.iv_base + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // host
            f.instruction(&Instruction::GlobalGet(2));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0));
            st64_local(f, vals, src.0);
            st8(f, state, 2);
            // fill：caching==1 语境（静态），写后缓存即镜像。
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::LocalSet(ic));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(iv));
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
            // host 臂（caching==0）同样镜像——icache 恒等于「本 op 刚
            // 写的值」，LoadVar 缓存臂读它不会陈旧。
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::LocalSet(ic));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(iv));
            f.instruction(&Instruction::End); // done
        }
        Instr::IncrVar { dst, var, amount, fail, site } => {
            let var = *var;
            if let Some(fc) = &e.fast {
                if let Some(slot) = fc.param_slot.get(&var) {
                    let tmp = fc.tmp64;
                    let sb = fc.slot_base;
                    let mut slot_addr = |f: &mut Function| {
                        f.instruction(&Instruction::LocalGet(sb));
                    };
                    f.instruction(&Instruction::Block(BlockType::Empty)); // done
                    f.instruction(&Instruction::Block(BlockType::Empty)); // slow
                    slot_addr(&mut *f);
                    f.instruction(&Instruction::I64Load(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    f.instruction(&Instruction::LocalSet(tmp));
                    f.instruction(&Instruction::LocalGet(tmp));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::I64And);
                    f.instruction(&Instruction::I64Eqz);
                    f.instruction(&Instruction::BrIf(0)); // 非 imm → slow
                    f.instruction(&Instruction::LocalGet(tmp));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::I64ShrS);
                    f.instruction(&Instruction::I64Const(*amount));
                    f.instruction(&Instruction::I64Add);
                    f.instruction(&Instruction::LocalSet(tmp));
                    f.instruction(&Instruction::LocalGet(tmp));
                    f.instruction(&Instruction::I64Const(62));
                    f.instruction(&Instruction::I64ShrS);
                    f.instruction(&Instruction::I64Eqz);
                    f.instruction(&Instruction::LocalGet(tmp));
                    f.instruction(&Instruction::I64Const(62));
                    f.instruction(&Instruction::I64ShrS);
                    f.instruction(&Instruction::I64Const(-1));
                    f.instruction(&Instruction::I64Eq);
                    f.instruction(&Instruction::I32Or);
                    f.instruction(&Instruction::I32Eqz);
                    f.instruction(&Instruction::BrIf(0)); // 越界 → slow
                    f.instruction(&Instruction::LocalGet(tmp));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::I64Shl);
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::I64Or);
                    f.instruction(&Instruction::LocalSet(dst.0));
                    slot_addr(&mut *f);
                    f.instruction(&Instruction::LocalGet(dst.0));
                    f.instruction(&Instruction::I64Store(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    fast_mask_set(&mut *f, fc.tmp64, fc.slot_base, *slot);
                    f.instruction(&Instruction::Br(1)); // done
                    f.instruction(&Instruction::End); // slow:
                    f.instruction(&Instruction::I32Const(e.name_handle(var)));
                    // incr_var 宿主闭包对增量做 untag（与常规 linker 同
                    // 约定）——发射端必须送 tagged 立即数，否则 amount 被
                    // 右移折半（inc1：incr i 5 实际加 2）。
                    f.instruction(&Instruction::I64Const(abi::tag_imm(*amount)));
                    f.instruction(&Instruction::Call(import::INCR_VAR));
                    f.instruction(&Instruction::LocalSet(dst.0));
                    check_sentinel(e, f, dst.0, *fail);
                    slot_addr(&mut *f);
                    f.instruction(&Instruction::LocalGet(dst.0));
                    f.instruction(&Instruction::I64Store(MemArg {
                        offset: (*slot * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                    fast_mask_set(&mut *f, fc.tmp64, fc.slot_base, *slot);
                    f.instruction(&Instruction::End); // done
                    return;
                }
            }
            let vals = cache::VALUES + var * 8;
            let state = cache::STATE + var;
            // M2：缓存有效臂（icache 持 tagged 值；imm 守卫保留——缓存
            // 可能是句柄）。失效的 slow 跳转落进既有 host 路径。
            let ic = e.ic_base + var;
            let iv = e.iv_base + var;
            f.instruction(&Instruction::Block(BlockType::Empty)); // done
            f.instruction(&Instruction::Block(BlockType::Empty)); // miss
            // 有效臂双门（同 LoadVar）：caching==1 且 iv==1——caching=0
            // 世界 incr 必须走宿主 CALL（interp 写 → trace/别名可观察）。
            f.instruction(&Instruction::GlobalGet(2)); // caching
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0)); // → miss
            f.instruction(&Instruction::LocalGet(iv));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(0)); // iv==0 → miss
            f.instruction(&Instruction::LocalGet(ic));
            f.instruction(&Instruction::LocalSet(dst.0));
            imm_guard_br_slow(f, dst.0);
            // n = untag + amount
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::I64Const(*amount));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(dst.0));
            range_guard_br(f, dst.0, 0);
            retag_in_place(f, dst.0);
            // cache 刷新 + 写落盘（保守：每写进 VALUES，STATE 置脏）。
            f.instruction(&Instruction::LocalGet(dst.0));
            f.instruction(&Instruction::LocalSet(ic));
            st64_local(f, vals, dst.0);
            st8(f, state, 2);
            f.instruction(&Instruction::Br(1)); // → done
            f.instruction(&Instruction::End); // miss:
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
            range_guard_br(f, dst.0, 0);
            retag_in_place(f, dst.0);
            st64_local(f, vals, dst.0);
            st8(f, state, 2);
            f.instruction(&Instruction::Br(1)); // → done（host/miss 兄弟块）
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
            // rtcl.call 恒置 caching=0 → 全部缓存失效（宿主可能改写
            // 任意变量；此后逐 op 宿主读，icache 由 LoadVar/StoreVar
            // host 臂重新镜像）。
            invalidate_int_caches(e, f);
            f.instruction(&Instruction::End); // done
        }
        Instr::CallSite { dst, cmd_id, argc, fail, site, srcs, cont_back, brk_depth, cont_depth, esc, esc_depth, .. } => {
            emit_flush(e, f, *fail);
            // ── fast-tier 直呼：epoch 门 + 槽缓存 → call_indirect；
            // 未命中 resolve_proc 冷融合（解析+代打+回填缓存）。
            // 统一 rc 分派：OK/RETURN=值；ERROR→fail_path（帧+哨兵）；
            // BREAK/CONTINUE/STALE/EXIT 经多值返回向上传播（零宿主）。
            if let Some(fc) = &e.fast {
                // lazy-flush：任何宿主可见边界前落盘脏槽——命令分派
                //（interp 按名读帧）与被调方 upvar 读都依赖 interp 帧
                // 为真值。先于 SCRATCH 写（flush 用 tmp 栈，无冲突）。
                emit_slot_flush(e, f, *fail);
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
                // kind 1/2 = level-1 break/continue：内层循环出口 br
                //（经典臂同款深度 +1——resolve 点只比经典多 $done 一层，
                // $fallback 已在其前闭合；v3.1 循环进 fast 后必须在单元
                // 内解析，单元返回会被 proc 边界转成 "outside of a loop"
                // 错，p05 实证）。
                if let Some(d) = brk_depth {
                    f.instruction(&Instruction::LocalGet(e.kind));
                    f.instruction(&Instruction::I32Const(1));
                    f.instruction(&Instruction::I32Eq);
                    f.instruction(&Instruction::BrIf(d + 2));
                }
                if let Some(d) = cont_depth {
                    f.instruction(&Instruction::LocalGet(e.kind));
                    f.instruction(&Instruction::I32Const(2));
                    f.instruction(&Instruction::I32Eq);
                    f.instruction(&Instruction::If(BlockType::Empty));
                    if *cont_back {
                        emit_fuel(e, f, *fail);
                    }
                    f.instruction(&Instruction::Br(d + 3));
                    f.instruction(&Instruction::End);
                }
                // kind 5 = Next 区域 continue 逃逸（eval 型 continue 穿
                // body 区——gen_for 的 eval continue 形状）：跳第一个
                // body 区祖先循环的 cont（经典臂 esc_depth 同款深度 +1
                // ——resolve 点多 $done 一层）；无祖先/解析失败 → 单元
                // 返回 CONTINUE（proc 边界按 loop_signal 语义转错）。
                // 此前 kind 5 无臂——ladder 不含 5，穿落到哨兵值继续
                // 执行（gen_for FAIL 根因）。
                if esc.is_some() {
                    f.instruction(&Instruction::LocalGet(e.kind));
                    f.instruction(&Instruction::I32Const(5));
                    f.instruction(&Instruction::I32Eq);
                    f.instruction(&Instruction::If(BlockType::Empty));
                    match esc_depth {
                        Some(d) => {
                            f.instruction(&Instruction::Br(d + 3));
                        }
                        None => {
                            ret_fast_sentinel(f, code::CONTINUE);
                        }
                    }
                    f.instruction(&Instruction::End);
                }
                // 剩余 kinds（无循环的 1/2、level>1 的 3/4、stale 6）
                // 单元返回。
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
                // ── 调用后槽重同步：被调方可能经 upvar/global 改本帧槽
                //（uv2：p2 的 upvar 写落 interp 帧槽，wasm 槽 stale）。
                // 每 var 一次 sync_var（名字读或 0）写回槽——用专用
                // TMP64 临时（fc.tmp64），不碰 dst（调用结果仍在途）。
                // 仅体含 CallSite 时发射（纯算术叶子无人可写本帧）。
                if fc.resync_on {
                // dirty 门：被调方实际执行了跨帧 upvar/global 才重拉
                //（宿主置位；此处检查+清除）。常见路径零宿主读。
                f.instruction(&Instruction::GlobalGet(6)); // dirty（fuel0 epoch1 caching2 cur_proc3 site_cache_base4 fast_depth5 dirty6）
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::GlobalSet(6));
                emit_resync_slots(e, f);
                }
                f.instruction(&Instruction::End); // dirty end
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
            // call/call_argv 闭包置 caching=0（命令可写任意变量）→
            // M2 缓存整表失效（须在哨兵分支之前——kind!=0 会 br 回循环）。
            invalidate_int_caches(e, f);
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
            } else {
                // kind 1 无循环可落（deferred `return -code break` 跨
                // proc 边界入单元——is_break 涵盖 deferred）：break 逃
                // 出单元 → rc BREAK（顶层 D-11 解码转错；pending 已被
                // take_fail 的早退消费，finish 由 rc+last_fail 重建）。
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::I32Const(code::BREAK));
                f.instruction(&Instruction::Return);
                f.instruction(&Instruction::End);
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
            } else {
                // kind 2 同上（无循环 → 单元 CONTINUE）。
                f.instruction(&Instruction::LocalGet(e.kind));
                f.instruction(&Instruction::I32Const(2));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::I32Const(code::CONTINUE));
                f.instruction(&Instruction::Return);
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
            let fused = e.fast.as_ref().and_then(|fc| fc.fused_cmp.get(&dst.0)).copied();
            // C-float（§5）：float lane on 时同 emit_arith 的三段式——
            // imm-imm → i64 单调比较；Float/混合 → f64 比较（IEEE754 =
            // C double，NaN 恒假 = tclsh）；其余 → 宿主。融合表三臂贯通：
            // int 快臂直写 b(br)、float 臂 raw cmp 直写 b(br)（真值与慢
            // 臂 (tagged≠1) 一致）、慢臂 call_binary_op 后补写。比较语境
            // exact_only=true——tclsh wide/double 比较精确（>2^53 imm 提
            // 升会收缩成邻值，9007199254740993>9007199254740992.0 实证），
            // 越界 imm 落宿主。imm 守卫 br 0 = 退出 intp → float path
            // （br 1 是 flt label → 宿主区，半成品误写，2026-11-18 修正）。
            if e.float_lane {
                f.instruction(&Instruction::Block(BlockType::Empty)); // done
                f.instruction(&Instruction::Block(BlockType::Empty)); // host
                f.instruction(&Instruction::Block(BlockType::Empty)); // flt
                f.instruction(&Instruction::Block(BlockType::Empty)); // intp
                f.instruction(&Instruction::LocalGet(a.0));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64And);
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::BrIf(0));
                f.instruction(&Instruction::LocalGet(b.0));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64And);
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::BrIf(0));
                f.instruction(&Instruction::LocalGet(a.0));
                f.instruction(&Instruction::LocalGet(b.0));
                f.instruction(match cc {
                    CmpOp::Lt => &Instruction::I64LtS,
                    CmpOp::Gt => &Instruction::I64GtS,
                    CmpOp::Le => &Instruction::I64LeS,
                    CmpOp::Ge => &Instruction::I64GeS,
                });
                if let Some(br) = fused {
                    f.instruction(&Instruction::LocalSet(e.b(br)));
                } else {
                    tag_bool(f, dst.0);
                }
                f.instruction(&Instruction::Br(3));
                f.instruction(&Instruction::End); // intp 终 → float path
                emit_float_operand(f, a.0, e.fa, true);
                emit_float_operand(f, b.0, e.fb, true);
                let fc = match cc {
                    CmpOp::Lt => BinOp::Lt,
                    CmpOp::Gt => BinOp::Gt,
                    CmpOp::Le => BinOp::Le,
                    CmpOp::Ge => BinOp::Ge,
                };
                emit_float_op(fc, dst.0, fused, e, f);
                f.instruction(&Instruction::Br(2));
                f.instruction(&Instruction::End); // flt 终 → host
                call_binary_op(
                    match cc {
                        CmpOp::Lt => BinOp::Lt,
                        CmpOp::Gt => BinOp::Gt,
                        CmpOp::Le => BinOp::Le,
                        CmpOp::Ge => BinOp::Ge,
                    },
                    a.0,
                    b.0,
                    dst.0,
                    *fail,
                    e,
                    f,
                );
                if let Some(br) = fused {
                    f.instruction(&Instruction::LocalGet(dst.0));
                    f.instruction(&Instruction::I64Const(1));
                    f.instruction(&Instruction::I64Ne);
                    f.instruction(&Instruction::LocalSet(e.b(br)));
                }
                f.instruction(&Instruction::End); // host 终（同 arith 半成品漏写补）
                f.instruction(&Instruction::End); // done
                return;
            }
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
            if let Some(br) = fused {
                // 融合：i32 比较结果直写 b 寄存器（免 tag_bool + BoolOfBool）。
                f.instruction(&Instruction::LocalSet(e.b(br)));
            } else {
                tag_bool(f, dst.0);
            }
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End); // slow:
            let op = match cc {
                CmpOp::Lt => BinOp::Lt,
                CmpOp::Gt => BinOp::Gt,
                CmpOp::Le => BinOp::Le,
                CmpOp::Ge => BinOp::Ge,
            };
            call_binary_op(op, a.0, b.0, dst.0, *fail, e, f);
            if let Some(br) = fused {
                // 慢路径结果仍须落到 b 寄存器（快路径已直写）——
                // BoolOfBool 转换语义（tagged bool 与 tag(0) 不等）。
                f.instruction(&Instruction::LocalGet(dst.0));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64Ne);
                f.instruction(&Instruction::LocalSet(e.b(br)));
            }
            f.instruction(&Instruction::End); // done
        }
        Instr::Unary { kind, dst, a, fail } => emit_unary(*kind, dst.0, a.0, *fail, e, f),
        Instr::BoolOfBool { dst, v } => {
            // 融合臂：Cmp fast 路径已直写 b 寄存器（慢路径也在 Cmp 臂
            // 补写）——此处空转。
            if e.fast.as_ref().is_some_and(|fc| fc.fused_cmp.contains_key(&v.0)) {
                return;
            }
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
            // fast：Start 宿主侧 bind(0) 前 flush——槽独占值（`set t 0`
            // 尚未落 interp）先落盘，随后的 resync 才不会把不存在变量
            // 的槽抹成 0（fe1 首轮实测）。
            if e.fast.is_some() {
                emit_slot_flush(e, f, *fail);
            }
            f.instruction(&Instruction::I32Const(*info as i32));
            f.instruction(&Instruction::I32Const(cache::SCRATCH as i32));
            f.instruction(&Instruction::I32Const(srcs.len() as i32));
            f.instruction(&Instruction::Call(import::FOREACH_START));
            f.instruction(&Instruction::LocalSet(e.scratch));
            // foreach_start 闭包置 caching=0（循环变量写 interp 帧）。
            invalidate_int_caches(e, f);
            check_sentinel(e, f, e.scratch, *fail);
            // iters = untag(result); i = 0。
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalSet(e.foreach_base + counter * 2 + 1));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(e.foreach_base + counter * 2));
            // fast：Start 宿主侧 bind(0)（lower 注释「解析 + bind(0)」）
            // 写迭代 0 的循环变量——背边 bind 的 resync 只覆盖迭代 ≥1，
            // 首迭代循环变量槽 stale（a1：外层二轮内层首读 j=旧值，
            // t=11 实证）。Start 后定向 resync 拉回循环变量。
            if let Some(fc) = &e.fast {
                emit_resync_foreach(f, fc, *info);
            }
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
            // fast：bind 前先落盘脏槽——bind 宿主侧要读/写 interp 帧
            //（循环变量），且紧随的 resync 按「interp 真值」覆盖全部
            // 槽：未 flush 的槽先被 sync_var 读成不存在→0 抹掉
            //（fe1 `set t 0` 槽值被抹 → can't read "t" 实证）。
            if e.fast.is_some() {
                emit_slot_flush(e, f, *fail);
            }
            f.instruction(&Instruction::I32Const(*info as i32));
            f.instruction(&Instruction::LocalGet(e.foreach_base + counter * 2));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::Call(import::FOREACH_BIND));
            f.instruction(&Instruction::LocalSet(e.scratch));
            // foreach_bind 闭包置 caching=0（bind 写帧变量）。
            invalidate_int_caches(e, f);
            check_sentinel(e, f, e.scratch, *fail);
            // fast：bind 宿主侧写了帧变量（循环变量）——fast 槽 stale，
            // 定向 sync 循环变量拉回（无此步循环体 LoadVar 读旧值，
            // fe 形状实证）。
            if let Some(fc) = &e.fast {
                emit_resync_foreach(f, fc, *info);
            }
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
            // eval 闭包置 caching=0（脚本可写任意变量）。
            invalidate_int_caches(e, f);
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::EvalExpr { dst, src, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::EVAL_EXPR));
            f.instruction(&Instruction::LocalSet(dst.0));
            invalidate_int_caches(e, f);
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::LappendVar { dst, src, var, fail } => {
            emit_flush(e, f, *fail);
            f.instruction(&Instruction::I32Const(e.name_handle(*var)));
            f.instruction(&Instruction::LocalGet(src.0));
            f.instruction(&Instruction::Call(import::LAPPEND_VAR));
            f.instruction(&Instruction::LocalSet(dst.0));
            // lappend 闭包置 caching=0（写帧变量）。
            invalidate_int_caches(e, f);
            check_sentinel(e, f, dst.0, *fail);
        }
        Instr::SetSite(k) => {
            // fast 体不发 SetSite：cur_site 无 fast 侧读者（错误路径走
            // fails 表 + fail id），纯负载（fl 循环 2×2 指令/迭代）。
            if e.fast.is_none() {
                f.instruction(&Instruction::I32Const(*k as i32));
                f.instruction(&Instruction::GlobalSet(e.site_global()));
            }
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
        Instr::FlowExit { code, fail } => {
            // 单元级裸 break/continue：记失败点（顶层 D-11 解码的帧
            // 快照源——宿主 finish 用 last_fail 的 site/bodies），返回
            // BREAK/CONTINUE。冷路径（循环 break/continue 走 Br）。
            f.instruction(&Instruction::I32Const(*fail as i32));
            f.instruction(&Instruction::Call(import::FAIL));
            f.instruction(&Instruction::Drop);
            if e.fast.is_some() {
                ret_fast_sentinel(f, *code);
            } else {
                f.instruction(&Instruction::I32Const(*code));
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
                // fast_impl 结果是 (i64, i32) 多值——return 前必须补
                // 值位（run 是单 i32，无需）。
                if e.fast.is_some() {
                    f.instruction(&Instruction::I64Const(0));
                }
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
    // binary_op 闭包置 caching=0 → M2 缓存整表失效（表达式可含
    // [cmd] 替换——宿主注释原文）。classic-only（fast 无 caching 语义）。
    invalidate_int_caches(e, f);
    check_sentinel(e, f, dst, fail);
}

/// M2 slot 缓存整表失效：caching=1→0 的宿主 import（binary_op/call/
/// call_argv/foreach_start/foreach_bind/eval_script/eval_expr/
/// lappend_var）执行后，interp 变量可能已被任意改写——所有 ivalid
/// 清零。此后 LoadVar/StoreVar 的 host 臂逐 op 重新镜像 icache。
/// classic-only（fast tier 无 caching 语义，e.fast 判别）。
fn invalidate_int_caches(e: &Env, f: &mut Function) {
    if e.fast.is_some() {
        return;
    }
    for var in 0..e.l2.vars.len() as u32 {
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(e.iv_base + var));
    }
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
    // C-float：Neg 三段式——Float rep → f64.neg（-0.0/NaN 语义 IEEE）；
    // imm int → int neg（`-5` 是 int 不是 float）；其余宿主。
    // imm 守卫 br 0 = 退出 intp → float path（同 arith/Cmp，2026-11-18
    // 修正半成品的 br 1）；范围守卫 BrIf(2) = intp 内 host。
    if e.float_lane && kind == BinOp::Neg {
        f.instruction(&Instruction::Block(BlockType::Empty)); // done
        f.instruction(&Instruction::Block(BlockType::Empty)); // host
        f.instruction(&Instruction::Block(BlockType::Empty)); // flt
        f.instruction(&Instruction::Block(BlockType::Empty)); // intp
        f.instruction(&Instruction::LocalGet(a));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::BrIf(0));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(a));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64ShrS);
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(e.scratch));
        // 范围守卫：失败落 host（intp 内深度 host=2）。
        range_guard_br(f, e.scratch, 2);
        f.instruction(&Instruction::LocalGet(e.scratch));
        f.instruction(&Instruction::LocalSet(dst));
        retag_in_place(f, dst);
        f.instruction(&Instruction::Br(3));
        f.instruction(&Instruction::End); // intp 终 → float path
        emit_float_operand(f, a, e.fa, false);
        f.instruction(&Instruction::LocalGet(e.fa));
        f.instruction(&Instruction::F64Neg);
        f.instruction(&Instruction::Call(import::BOX_FLOAT));
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::LocalSet(dst));
        f.instruction(&Instruction::Br(2));
        f.instruction(&Instruction::End); // flt 终 → host
        call_binary_op(kind, a, a, dst, fail, e, f);
        f.instruction(&Instruction::End); // host 终（同 arith 半成品漏写补）
        f.instruction(&Instruction::End); // done
        return;
    }
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
            range_guard_br(f, e.scratch, 0);
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
            range_guard_br(f, e.scratch, 0);
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

/// C-float（§5）可走 f64 内联的二元集：加减乘除（IEEE 直算）+ 全比较
/// （NaN/Inf = IEEE754 = C double）。pow 走宿主（transcendental 永远
/// 宿主，ULP 单一事实来源）；div-int 的 floor 语义留在 int 快臂。
fn float_capable(kind: BinOp) -> bool {
    matches!(
        kind,
        BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::Le
            | BinOp::Ge
    )
}

fn emit_arith(kind: BinOp, dst: u32, a: u32, b: u32, fail: u32, e: &Env, f: &mut Function) {
    // 全走宿主的 §4 类别：pow、字符串比较（div/mod 件5 起双 imm 快臂
    // 内联——floor 语义 wasm 直算，除零/非 imm 落宿主）。
    if matches!(kind, BinOp::Pow | BinOp::StrEq | BinOp::StrNe) {
        call_binary_op(kind, a, b, dst, fail, e, f);
        return;
    }
    // C-float 三段式（jit-float-lane on）：imm-imm → int 快臂；Float
    // rep / imm-int 混合 → f64 内联（convert 提升）；其余（串 shimmer
    // /bignum/整值句柄）→ binary_op 宿主。类型分派先于 opcode：
    // `1/2` 走 int floor = 0，`1./2` 走 f64 = 0.5（§5 语义清单）。
    // imm 守卫 br 0：退出 intp 即 float path 入口（intp 的 End 之后）；
    // br 1 是 flt 的 label → 落宿主区（前 agent 半成品误写，float path
    // 因此死码——2026-11-18 修正）。守卫失败（溢出/除零）经 slow_br=1
    // 落宿主全语义。
    if e.float_lane && float_capable(kind) {
        let exact = matches!(kind, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge);
        f.instruction(&Instruction::Block(BlockType::Empty)); // done
        f.instruction(&Instruction::Block(BlockType::Empty)); // host
        f.instruction(&Instruction::Block(BlockType::Empty)); // flt
        f.instruction(&Instruction::Block(BlockType::Empty)); // intp
        // imm 守卫：失败（非 imm）→ br 0 = 退出 intp → float path。
        f.instruction(&Instruction::LocalGet(a));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::BrIf(0));
        f.instruction(&Instruction::LocalGet(b));
        f.instruction(&Instruction::I64Const(1));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::BrIf(0));
        emit_int_inline(kind, dst, a, b, 1, e, f);
        retag_in_place(f, dst);
        // intp 内深度：intp=0, flt=1, host=2, done=3。
        f.instruction(&Instruction::Br(3));
        f.instruction(&Instruction::End); // intp 终 → 此处即 float path 入口
        emit_float_operand(f, a, e.fa, exact);
        emit_float_operand(f, b, e.fb, exact);
        emit_float_op(kind, dst, None, e, f);
        // float path 裸区深度：flt=0, host=1, done=2。
        f.instruction(&Instruction::Br(2));
        f.instruction(&Instruction::End); // flt 终 → host 兜底
        call_binary_op(kind, a, b, dst, fail, e, f);
        f.instruction(&Instruction::End); // host 终（半成品漏写——4 开 3 闭，
        // wasm 校验 "control frames remain"，2026-11-18 补）
        f.instruction(&Instruction::End); // done
        return;
    }
    f.instruction(&Instruction::Block(BlockType::Empty)); // done
    f.instruction(&Instruction::Block(BlockType::Empty)); // slow
    imm_guard_br_slow(f, a);
    imm_guard_br_slow(f, b);
    emit_int_inline(kind, dst, a, b, 0, e, f);
    retag_in_place(f, dst);
    f.instruction(&Instruction::Br(1));
    f.instruction(&Instruction::End); // slow:
    call_binary_op(kind, a, b, dst, fail, e, f);
    f.instruction(&Instruction::End); // done
}

/// int 快臂主体（无壳：守卫/retag/兜底脚手架由调用方负责）。
///
/// `slow_br` = 守卫失败落宿主的 br 深度：classic [done, slow] 内 = 0
/// （退出 slow → 宿主）；float-lane [done, host, flt, intp] 内 = 1
/// （退出 intp+flt → 宿主）。**所有守卫一律经此参数**——历史上 Mul
/// 守卫硬编码 br 1（classic 里 = done，跳过宿主且 dst 不写），
/// `3037000499*7` 实证产出陈旧 dst（2026-11-18 C-float 差分首曝）。
fn emit_int_inline(kind: BinOp, dst: u32, a: u32, b: u32, slow_br: u32, e: &Env, f: &mut Function) {
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
            range_guard_br(f, e.scratch, slow_br);
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
        }
        BinOp::Mul => {
            // §4：高 32 位符号扩展检查（|a|,|b| < 2^31）→ 积 < 2^62。
            // 守卫失败（|v| ≥ 2^31，如 3037000499）→ slow_br（宿主）。
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
                f.instruction(&Instruction::BrIf(slow_br));
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
            // （Shl/Shr 非 float_capable，只在 classic 布局可达；深度仍
            // 走 slow_br 便于统一——If 内再 +1。）
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
            f.instruction(&Instruction::Br(slow_br + 1)); // → slow（宿主）
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
                range_guard_br(f, e.scratch, slow_br);
            }
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::LocalSet(dst));
        }
        BinOp::Div | BinOp::Mod => {
            // 件5 双 imm 快臂：floor 语义 wasm 直算（expr.rs floor_div/
            // floor_mod 同构）。除零 → 宿主（"divide by zero" 全语义；
            // float-lane 里落 float 路径会算成 Inf/NaN——7/0 必须报错，
            // 故除零守卫同样走 slow_br）。
            // 操作数 imm ⇒ |v| < 2^62 ⇒ INT_MIN/-1 陷阱不可达；商修正只
            // 减（|q| ≤ |va|）而 |r+vb| < |vb| ≤ 2^62-1 ⇒ 结果恒在 i63
            // 域，免范围守卫——慢出口只剩 imm 守卫与除零，此后对 dst
            // 直写安全（慢臂重新读 a/b 局部）。
            // scratch = vb（守卫后就地复用）。
            f.instruction(&Instruction::LocalGet(b));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalSet(e.scratch));
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::BrIf(slow_br)); // vb == 0 → 宿主
            // dst = 截断商/余数
            f.instruction(&Instruction::LocalGet(a));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(if kind == BinOp::Div {
                &Instruction::I64DivS
            } else {
                &Instruction::I64RemS
            });
            f.instruction(&Instruction::LocalSet(dst));
            // floor 修正门：r != 0 && (r ^ vb) < 0（符号异）——两个门
            // **串联**（r==0 臂空体即闭，符号臂随后；嵌套会让修正只在
            // r==0 世界可达——b07 首版实证）。
            // Div 语境 r = va - q*vb（截断恒等式，免第二次除/余）。
            if kind == BinOp::Div {
                f.instruction(&Instruction::LocalGet(a));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64ShrS);
                f.instruction(&Instruction::LocalGet(dst));
                f.instruction(&Instruction::LocalGet(e.scratch));
                f.instruction(&Instruction::I64Mul);
                f.instruction(&Instruction::I64Sub);
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::If(BlockType::Empty)); // r==0
                f.instruction(&Instruction::End); // r==0 臂闭（空体）
            } else {
                f.instruction(&Instruction::LocalGet(dst));
                f.instruction(&Instruction::I64Eqz);
                f.instruction(&Instruction::If(BlockType::Empty)); // r==0
                f.instruction(&Instruction::End); // r==0 臂闭（空体）
            }
            f.instruction(&Instruction::LocalGet(dst));
            f.instruction(&Instruction::LocalGet(e.scratch));
            f.instruction(&Instruction::I64Xor);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            if kind == BinOp::Div {
                // floor_div：q - 1
                f.instruction(&Instruction::LocalGet(dst));
                f.instruction(&Instruction::I64Const(1));
                f.instruction(&Instruction::I64Sub);
                f.instruction(&Instruction::LocalSet(dst));
            } else {
                // floor_mod：r + vb
                f.instruction(&Instruction::LocalGet(dst));
                f.instruction(&Instruction::LocalGet(e.scratch));
                f.instruction(&Instruction::I64Add);
                f.instruction(&Instruction::LocalSet(dst));
            }
            f.instruction(&Instruction::End);
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
}
