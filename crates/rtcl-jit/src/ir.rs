//! Nanopass IR 层级（设计文档 §7.3）: L1（lane 标注）、L2（分配决策）、
//! L3（结构化 wasm 指令 IR）。L0 = `rtcl_ir::ByteCode`（现有）、L4 = wasm
//! 字节（wasm-encoder 输出）。每层 `Display` 可 golden 测试；`check::`
//! 提供逐层校验器（cpcheck 模式）。

use crate::abi::BinOp;
use rtcl_ir::opcode::OpCode;
use std::fmt;

// ── L1 ─────────────────────────────────────────────────────────────────────

/// 操作数/结果的静态 lane 假设（`annotate_lane` 产出）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// 确定是 tagged 立即数（来自 PushInt/PushTrue/PushFalse 等常量）。
    Imm,
    /// 运行时未知（变量读、宿主调用结果、…）。
    Any,
}

/// 一条标注后的 L1 指令。
#[derive(Debug, Clone, PartialEq)]
pub struct L1Op {
    pub op: OpCode,
    /// 两个输入槽的 lane（不足两位时后位填 [`Lane::Any`]）。
    pub in_lanes: [Lane; 2],
    pub out_lane: Lane,
    /// `plan_guards` 是否给该 op 计划了双立即数守卫（false = 全走宿主
    /// 或纯常量，无需守卫）。
    pub guarded: bool,
}

/// L1 = L0 + lane 标注（每 op 操作数假设 + 守卫点位）。
#[derive(Debug, Clone, PartialEq)]
pub struct L1 {
    pub ops: Vec<L1Op>,
}

impl fmt::Display for L1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "L1 ({} ops):", self.ops.len())?;
        for (i, a) in self.ops.iter().enumerate() {
            let lane = |l: Lane| match l {
                Lane::Imm => "imm",
                Lane::Any => "any",
            };
            writeln!(
                f,
                "{:04} {:<28} in=[{},{}] out={} guard={}",
                i,
                a.op,
                lane(a.in_lanes[0]),
                lane(a.in_lanes[1]),
                lane(a.out_lane),
                a.guarded
            )?;
        }
        Ok(())
    }
}

// ── L2 ─────────────────────────────────────────────────────────────────────

/// 一个被引用的变量名常量（`escape` 收集，`alloc_locals` 定槽）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarSlot {
    /// 常量池索引；宿主侧句柄 = `CONST_BASE + idx`。
    pub const_idx: u32,
    /// 常量池中的名字文本。
    pub name: String,
    /// 变量缓存的内存下标（`abi::cache`）。
    pub cache: u32,
}

/// 回边（target < source 的转移）——fuel/epoch 织入点 + 其 fuel 失败
/// 上下文（alloc_locals 同时为每个回边追加的 FailInfo 的表索引）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backedge {
    pub src: u32,
    pub target: u32,
    pub fail: u32,
}

/// 宿主调用失败点的静态上下文（编译期模拟烘焙；运行时 bodies/cur_site
/// 在结构化 ByteCode 上是路径不变的）。
#[derive(Debug, Clone, PartialEq)]
pub struct FailInfo {
    /// 出错 op 的下标（诊断用）。
    pub pc: u32,
    /// 当时 cur_site（BeginCmd 语境）。
    pub site: u32,
    /// 当时开着的 region 栈（内层在末尾）——`unwind_subs` 的驱动侧复刻
    /// 需要它。`(kind, site)` 对。
    pub bodies: Vec<(BodyKind, u32)>,
    /// 词操作（LoadVar）——M1' 中 err_from_subst 恒 false，保留协议位。
    pub word_op: bool,
}

/// 内联 region 种类（vm_exec `Region` 的静态子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Body,
    Next,
    Sub,
    Expr,
}

impl fmt::Display for BodyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BodyKind::Body => "body",
            BodyKind::Next => "next",
            BodyKind::Sub => "sub",
            BodyKind::Expr => "expr",
        })
    }
}

/// L2 = L1 + slot→wasm-local/内存分配决策。
#[derive(Debug, Clone, PartialEq)]
pub struct L2 {
    pub ops: Vec<L1Op>,
    /// 每条 op 的输入栈槽（值寄存器下标）与输出槽（若有）。
    pub reads: Vec<Vec<u32>>,
    pub writes: Vec<Option<u32>>,
    /// 值栈最大深度 = i64 寄存器个数。
    pub max_depth: u32,
    /// 变量缓存槽（按首次引用顺序）。
    pub vars: Vec<VarSlot>,
    /// 宿主失败点表（`rtcl.fail(fail_id)` 的 fail_id 下标）。
    pub fails: Vec<FailInfo>,
    /// 每个「会失败的宿主交互」op 的 fail 表索引（无则 None）——
    /// 与 `ops` 平行。
    pub fail_of: Vec<Option<u32>>,
    /// 常量池中被引用的字符串（句柄 = CONST_BASE + 常量池下标）。
    pub used_consts: Vec<u32>,
    /// 控制流边（结构化用）：`edges[i]` = op i 的后继 pc 集。
    pub edges: Vec<Vec<u32>>,
    /// 回边（target < source 的转移）——fuel/epoch 织入点。
    pub backedges: Vec<Backedge>,
    /// 值池空值/常量基址（见 `abi`）。
    pub const_base: u32,
}

impl fmt::Display for L2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "L2: {} ops, depth {}, {} vars, {} fails, backedges {:?}",
            self.ops.len(),
            self.max_depth,
            self.vars.len(),
            self.fails.len(),
            self.backedges
        )?;
        for (i, a) in self.ops.iter().enumerate() {
            writeln!(
                f,
                "{:04} {:<28} r={:?} w={:?} fail={:?}",
                i,
                a.op,
                self.reads[i],
                self.writes[i],
                self.fail_of[i]
            )?;
        }
        if !self.vars.is_empty() {
            writeln!(f, "vars:")?;
            for v in &self.vars {
                writeln!(f, "  cache={} {} (const {})", v.cache, v.name, v.const_idx)?;
            }
        }
        for (i, fi) in self.fails.iter().enumerate() {
            let bodies: Vec<String> =
                fi.bodies.iter().map(|(k, s)| format!("{k}:{s}")).collect();
            writeln!(
                f,
                "fail[{i}] pc={} site={} word={} bodies=[{}]",
                fi.pc,
                fi.site,
                fi.word_op,
                bodies.join(" ")
            )?;
        }
        Ok(())
    }
}

// ── L3 ─────────────────────────────────────────────────────────────────────

/// 值寄存器（tagged i64）与布尔寄存器（i32）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BReg(pub u32);

/// 结构化指令树。`target` = 待解析的目标 pc（lower 构造）；`depth` 由
/// phase B 按标签栈解析后生效；`backedge` 由 lower 依据目标回边判定
/// （`instrument` 只读该标记）。
#[derive(Debug, Clone, PartialEq)]
pub enum L3 {
    Seq(Vec<L3>),
    /// `cond != 0` → then，否则 else；`j` = 汇合点 pc（自身的 br 标签）。
    If { cond: BReg, j: u32, then_: Box<L3>, else_: Box<L3> },
    /// header = 回边目标 pc（loop 标签），exit = LoopEnter 的 brk（外层
    /// block 标签）。
    Loop { header: u32, exit: u32, body: Box<L3> },
    /// 显式标签块（前向跳过区域的续体标签）。
    Block { cont: u32, body: Box<L3> },
    Br { target: u32, depth: u32, backedge: bool, fail: u32 },
    BrIf { target: u32, depth: u32, cond: BReg, when_zero: bool, backedge: bool, fail: u32 },
    Instr(Instr),
}

/// 语义模板（lower 产出；assemble 展开为 wasm 指令序列——快车道模板 +
/// 兜底 import 模板，§7.3）。
#[derive(Debug, Clone, PartialEq)]
pub enum Instr {
    /// tagged 常量（立即数或预驻留句柄，bits 直接烘焙）。
    Const { dst: Reg, bits: i64, what: ConstWhat },
    Copy { dst: Reg, src: Reg },
    /// PushFloat：box_float 宿主装箱（float lane off，C-float 里程碑）。
    BoxFloat { dst: Reg, v: f64 },
    /// 变量/常量/宿主交互模板（展开含守卫与哨兵检查）。
    LoadVar { dst: Reg, var: u32, fail: u32 },
    StoreVar { src: Reg, var: u32, keep: bool, fail: u32 },
    IncrVar { dst: Reg, var: u32, amount: i64, fail: u32 },
    /// §4 快车道二元运算（guard: 双立即数；失败 → binary_op 兜底）。
    Arith { kind: BinOp, dst: Reg, a: Reg, b: Reg, fail: u32 },
    /// §4 比较免 untag：tagged word 上 i64 比较。
    Cmp { cc: CmpOp, dst: Reg, a: Reg, b: Reg, fail: u32 },
    /// 一元（Neg/Not/BitNot）。
    Unary { kind: BinOp, dst: Reg, a: Reg, fail: u32 },
    /// strict_bool 兜底（JumpTrue/JumpFalse 的 TOS）→ 0/1 布尔寄存器。
    BoolTest { dst: BReg, v: Reg, fail: u32 },
    /// 产生者已知是 tagged 0/1（Cmp/Eq/Ne/And/Or/Not/StrEq/StrNe 的
    /// 输出恒为 tag(0)/tag(1)）——纯转换，无宿主路径。
    BoolOfBool { dst: BReg, v: Reg },
    /// 命令分派（宿主 eval 路由；含缓存 flush 与 break/continue 出口）。
    /// `brk`/`cont` = 出口目标 pc（无循环 = None）；`cont_back` = continue
    /// 目标是回边（需 fuel 计步）；phase B 会把目标 pc 改写为分支深度。
    CallSite {
        dst: Reg,
        cmd_id: u16,
        argc: u16,
        fail: u32,
        /// 站点索引（宿主 eval 路由用——静态正确，见 fail 表）。
        site: u32,
        brk: Option<u32>,
        cont: Option<u32>,
        cont_back: bool,
        /// phase B 解析后的出口深度（None = 无循环或不可达标签）。
        brk_depth: Option<u32>,
        cont_depth: Option<u32>,
    },
    /// `rtcl.fail(fail_id)` + 返回 TCL_ERROR。
    Fail { fail: u32 },
    SetResult { src: Reg },
    ReturnCode { code: i32 },
    /// Exit op → 宿主终局（driver 复刻 rtcl 文件模式行为）。
    Exit { code: i32 },
    /// BeginCmd：模块私有 `cur_site` global 更新（宿主 call 路由需要
    /// 当前命令站点文本/行号）。
    SetSite(u32),
    /// `instrument` 织入：fuel 自减 + refuel + 哨兵检查。
    BackedgeFuel { fail: u32 },
    /// `instrument` 织入：入口快照/回边比对 epoch，不等 → STALE。
    EpochCheck { at_entry: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstWhat {
    Imm,
    Handle,
}

/// 数值比较种类（§4 表）。tagged word 上直接 `i64.lt_s` 等即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Gt,
    Le,
    Ge,
}

impl fmt::Display for L3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_l3(self, f, 0)
    }
}

fn pad(n: usize) -> String {
    "  ".repeat(n)
}

fn fmt_l3(n: &L3, f: &mut fmt::Formatter<'_>, ind: usize) -> fmt::Result {
    match n {
        L3::Seq(items) => {
            writeln!(f, "{}seq", pad(ind))?;
            for i in items {
                fmt_l3(i, f, ind + 1)?;
            }
            Ok(())
        }
        L3::If { cond, j, then_, else_ } => {
            writeln!(f, "{}if b{} (j={})", pad(ind), cond.0, j)?;
            fmt_l3(then_, f, ind + 1)?;
            writeln!(f, "{}else", pad(ind))?;
            fmt_l3(else_, f, ind + 1)?;
            writeln!(f, "{}end", pad(ind))
        }
        L3::Loop { header, exit, body } => {
            writeln!(f, "{}loop hdr={} exit={}", pad(ind), header, exit)?;
            fmt_l3(body, f, ind + 1)?;
            writeln!(f, "{}end", pad(ind))
        }
        L3::Block { cont, body } => {
            writeln!(f, "{}block cont={}", pad(ind), cont)?;
            fmt_l3(body, f, ind + 1)?;
            writeln!(f, "{}end", pad(ind))
        }
        L3::Br { depth, backedge, .. } => {
            writeln!(f, "{}br {}{}", pad(ind), depth, if *backedge { " (backedge)" } else { "" })
        }
        L3::BrIf { depth, cond, when_zero, backedge, .. } => {
            writeln!(
                f,
                "{}br_if {} b{} when{}{}",
                pad(ind),
                depth,
                cond.0,
                if *when_zero { "=0" } else { "!=0" },
                if *backedge { " (backedge)" } else { "" }
            )
        }
        L3::Instr(i) => writeln!(f, "{}{:?}", pad(ind), i),
    }
}
