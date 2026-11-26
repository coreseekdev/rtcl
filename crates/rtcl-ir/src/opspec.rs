//! OpSpec — the instruction set's single source of truth (D2 第一步).
//!
//! One row per [`OpCode`] variant: the mnemonic (disassembly/error text),
//! the operand kinds and widths (payload fields in declaration order), the
//! C2' top-level slot boundary class, whether the op opens a command site
//! (the 行号/命令文本站点), whether the compiler may emit it (the fallback
//! whitelist), and the fused op's canonical expansion (`lowers_to`).
//!
//! Consumers that used to carry their own per-op tables all read this one:
//! `Display` (反汇编), `exec_inner`'s two classification matches
//! (`begin_cmd_site`/`top_op_class`), `unfused()` + `has_fused_ops` (the
//! JIT/rtcl-vm boundary), the compiler's fallback whitelist, and the
//! bytecode validator's operand checks.
//!
//! Adding a superinstruction therefore touches: the enum variant, the
//! `decompose`/`compose` arms, one row here, the peephole emission rule,
//! and the executor handler — every metadata consumer follows the row.
//! The census test at the bottom enforces the decompose/compose/row
//! three-way agreement (uniqueness, round-trip, byte-exact Display).

use crate::opcode::OpCode;

/// Dense variant index — the join between the enum and [`OPS`].
pub type OpIdx = u16;

/// Operand kinds — the role and width of one payload field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opnd {
    /// u16 constant-pool index.
    Const16,
    /// u32 constant-pool index (wide pushes).
    Const32,
    /// u16 call-frame slot.
    Slot16,
    /// u32 call-frame slot (wide locals).
    Slot32,
    /// i64 signed immediate.
    Imm64,
    /// i32 signed immediate.
    Imm32,
    /// u32 absolute jump/handler target (instruction index).
    Target32,
    /// u32 command-site index (`ByteCode::sites`; the 行号站点).
    Site32,
    /// u32 foreach-info table index.
    Info32,
    /// u16 count (argc / concat width).
    Count16,
    /// u32 count (wide argc).
    Count32,
    /// u8 comparison code (0=<, 1=>, 2=<=, 3=>=, 4===, 5=!=).
    Cc8,
    /// i32 return/exit code.
    Code32,
    /// f64 literal.
    F64,
    /// u16 upvar level.
    Level16,
    /// raw u32 annotation (LINE's line number) — not a site/target.
    Note32,
}

/// One payload field: its [`Opnd`] kind plus the disassembly decoration
/// (prefix like `cc=` / `argc=`, suffix like the `[...]` bracket form).
#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub pre: &'static str,
    pub post: &'static str,
    pub kind: Opnd,
}

const fn a(kind: Opnd) -> Arg {
    Arg { pre: "", post: "", kind }
}

const fn p(kind: Opnd, pre: &'static str) -> Arg {
    Arg { pre, post: "", kind }
}

/// C2' top-level slot boundary class (the old `top_op_class` match).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    /// Pure — zero global touch; the top-slot cache stays valid.
    Pure,
    /// Name-keyed read — flush (ts → 全局) then read; no invalidation.
    FlushOnly,
    /// Dispatch / name write / eval — flush + invalidate all slots.
    FlushInvalidate,
}

/// One canonical op of a fused op's expansion.
#[derive(Debug, Clone, Copy)]
pub struct Lower {
    /// Which op to build — a fixed index, or the comparison selected by
    /// a `cc` source field (0=<, 1=>, 2=<=, 3=>=, 4===, else !=).
    pub op: OpSel,
    /// Operand sources, in the target's field order (missing fields = 0).
    pub args: &'static [LSel],
}

#[derive(Debug, Clone, Copy)]
pub enum OpSel {
    Idx(OpIdx),
    CmpOf(u8),
}

/// Operand source for an expansion step: a source field (copied,
/// sign-extended i32→i64, or +1), or a constant.
#[derive(Debug, Clone, Copy)]
pub enum LSel {
    /// Copy source field k verbatim.
    F(u8),
    /// Source field k as i64 (sign-extend an i32 field).
    FI64(u8),
    /// Constant u16.
    K16(u16),
    /// Constant i64.
    K64(i64),
}

/// Per-variant metadata row — see the crate docs for the consumer list.
#[derive(Debug)]
pub struct OpSpec {
    /// Mnemonic used by disassembly and error text.
    pub name: &'static str,
    /// Payload fields in declaration order.
    pub args: &'static [Arg],
    /// Top-level slot boundary class.
    pub top: OpClass,
    /// Opens a command site: field 0 is a Site32 operand (BeginCmd family).
    pub site_open: bool,
    /// The compiler may emit this op (the fallback whitelist).
    pub emittable: bool,
    /// A peephole fusion product (the JIT/rtcl-vm boundary expands these).
    pub fused: bool,
    /// Canonical expansion; empty = not fused (copy verbatim).
    pub lowers: &'static [Lower],
}

impl OpSpec {
    /// Whether field `k` is a jump/handler target (remap/bounds consumers).
    pub fn is_target_field(&self, k: usize) -> bool {
        self.args.get(k).map(|arg| arg.kind == Opnd::Target32).unwrap_or(false)
    }
}

/// Dense indices — stable handles for the lowers targets and consumers.
pub mod ix {
    use super::OpIdx;
    pub const PUSH_CONST: OpIdx = 0;
    pub const PUSH_CONST_WIDE: OpIdx = 1;
    pub const PUSH_EMPTY: OpIdx = 2;
    pub const PUSH_INT: OpIdx = 3;
    pub const PUSH_FLOAT: OpIdx = 4;
    pub const PUSH_TRUE: OpIdx = 5;
    pub const PUSH_FALSE: OpIdx = 6;
    pub const POP: OpIdx = 7;
    pub const DUP: OpIdx = 8;
    pub const LOAD_VAR: OpIdx = 9;
    pub const STORE_VAR: OpIdx = 10;
    pub const STORE_VAR_POP: OpIdx = 11;
    pub const LOAD_LOCAL: OpIdx = 12;
    pub const STORE_LOCAL: OpIdx = 13;
    pub const STORE_LOCAL_POP: OpIdx = 14;
    pub const LOAD_ARRAY_ELEM: OpIdx = 15;
    pub const STORE_ARRAY_ELEM: OpIdx = 16;
    pub const INCR_VAR: OpIdx = 17;
    pub const INCR_VAR_POP: OpIdx = 18;
    pub const INCR_LOCAL: OpIdx = 19;
    pub const INCR_LOCAL_POP: OpIdx = 20;
    pub const APPEND_VAR: OpIdx = 21;
    pub const UNSET_VAR: OpIdx = 22;
    pub const VAR_EXISTS: OpIdx = 23;
    pub const PUSH_FRAME: OpIdx = 24;
    pub const POP_FRAME: OpIdx = 25;
    pub const UPVAR: OpIdx = 26;
    pub const GLOBAL: OpIdx = 27;
    pub const JUMP: OpIdx = 28;
    pub const JUMP_TRUE: OpIdx = 29;
    pub const JUMP_FALSE: OpIdx = 30;
    pub const LOOP_ENTER: OpIdx = 31;
    pub const LOOP_EXIT: OpIdx = 32;
    pub const FOREACH_START: OpIdx = 33;
    pub const FOREACH_NEXT: OpIdx = 34;
    pub const FOREACH_COLLECT: OpIdx = 35;
    pub const FOREACH_END: OpIdx = 36;
    pub const BREAK: OpIdx = 37;
    pub const CONTINUE: OpIdx = 38;
    pub const RETURN: OpIdx = 39;
    pub const RETURN_CODE: OpIdx = 40;
    pub const EXIT: OpIdx = 41;
    pub const ADD: OpIdx = 42;
    pub const SUB: OpIdx = 43;
    pub const LAPPEND_L: OpIdx = 44;
    pub const LAPPEND_V: OpIdx = 45;
    pub const LAPPEND_VV: OpIdx = 46;
    pub const LAPPEND_SS: OpIdx = 47;
    pub const BODY_MARK_FRAMED: OpIdx = 48;
    pub const ADD_SS: OpIdx = 49;
    pub const ADD_SI: OpIdx = 50;
    pub const CMP_SS: OpIdx = 51;
    pub const CMP_SI: OpIdx = 52;
    pub const ADD_VV: OpIdx = 53;
    pub const ADD_VI: OpIdx = 54;
    pub const CMP_VV: OpIdx = 55;
    pub const CMP_VI: OpIdx = 56;
    pub const CMP_SS_JT: OpIdx = 57;
    pub const CMP_SS_JF: OpIdx = 58;
    pub const CMP_SI_JT: OpIdx = 59;
    pub const CMP_SI_JF: OpIdx = 60;
    pub const CMP_VV_JT: OpIdx = 61;
    pub const CMP_VV_JF: OpIdx = 62;
    pub const CMP_VI_JT: OpIdx = 63;
    pub const CMP_VI_JF: OpIdx = 64;
    pub const MUL: OpIdx = 65;
    pub const DIV: OpIdx = 66;
    pub const MOD: OpIdx = 67;
    pub const POW: OpIdx = 68;
    pub const NEG: OpIdx = 69;
    pub const EQ: OpIdx = 70;
    pub const NE: OpIdx = 71;
    pub const LT: OpIdx = 72;
    pub const GT: OpIdx = 73;
    pub const LE: OpIdx = 74;
    pub const GE: OpIdx = 75;
    pub const STR_EQ: OpIdx = 76;
    pub const STR_NE: OpIdx = 77;
    pub const AND: OpIdx = 78;
    pub const OR: OpIdx = 79;
    pub const NOT: OpIdx = 80;
    pub const BIT_AND: OpIdx = 81;
    pub const BIT_OR: OpIdx = 82;
    pub const BIT_XOR: OpIdx = 83;
    pub const BIT_NOT: OpIdx = 84;
    pub const SHL: OpIdx = 85;
    pub const SHR: OpIdx = 86;
    pub const CONCAT: OpIdx = 87;
    pub const MAKE_LIST: OpIdx = 88;
    pub const LIST_APPEND: OpIdx = 89;
    pub const LIST_INDEX: OpIdx = 90;
    pub const LIST_LENGTH: OpIdx = 91;
    pub const STR_LEN: OpIdx = 92;
    pub const STR_INDEX: OpIdx = 93;
    pub const EXPAND_LIST: OpIdx = 94;
    pub const EXPAND_MARK: OpIdx = 95;
    pub const CALL: OpIdx = 96;
    pub const CALL_EXPAND: OpIdx = 97;
    pub const DYNCALL: OpIdx = 98;
    pub const DYNCALL_EXPAND: OpIdx = 99;
    pub const CALL_PROC: OpIdx = 100;
    pub const TAIL_CALL_PROC: OpIdx = 101;
    pub const EVAL_SCRIPT: OpIdx = 102;
    pub const EVAL_EXPR: OpIdx = 103;
    pub const CATCH_START: OpIdx = 104;
    pub const CATCH_END: OpIdx = 105;
    pub const LINE: OpIdx = 106;
    pub const BEGIN_CMD: OpIdx = 107;
    pub const BEGIN_CMD_SUB: OpIdx = 108;
    pub const EXPR_MARK_BEGIN: OpIdx = 109;
    pub const BEGIN_CMD_PUSH_CONST: OpIdx = 110;
    pub const SUB_END_STORE_L_POP: OpIdx = 111;
    pub const SUB_END_STORE_V_POP: OpIdx = 112;
    pub const SUB_END_RETURN: OpIdx = 113;
    pub const SWAP_TO_NEXT_MARK: OpIdx = 114;
    pub const NEXT_MARK_BEGIN_CMD: OpIdx = 115;
    pub const BEGIN_CMD_INCR_L_POP: OpIdx = 116;
    pub const BEGIN_CMD_INCR_V_POP: OpIdx = 117;
    pub const FOR_NEXT_INCR_L: OpIdx = 118;
    pub const FOR_NEXT_INCR_V: OpIdx = 119;
    pub const FOREACH_NEXT_CLOSE: OpIdx = 120;
    pub const SET_SS_ADD: OpIdx = 121;
    pub const SET_VV_ADD: OpIdx = 122;
    pub const PUSH_CONST_SUB: OpIdx = 123;
    pub const SUB_MARK_BEGIN: OpIdx = 124;
    pub const EXPR_MARK_BEGIN_PUSH: OpIdx = 125;
    pub const SUB_SI: OpIdx = 126;
    pub const SUB_SS: OpIdx = 127;
    pub const SUB_END_DYNCALL: OpIdx = 128;
    pub const SUB_END_CALL: OpIdx = 129;
    pub const SUB_END_ADD: OpIdx = 130;
    pub const INCR_V_STACK: OpIdx = 131;
    pub const SUB_END_INCR_V_STACK: OpIdx = 132;
    pub const INCR_L_STACK: OpIdx = 133;
    pub const SUB_END_INCR_L_STACK: OpIdx = 134;
    pub const BODY_MARK: OpIdx = 135;
    pub const NEXT_MARK: OpIdx = 136;
    pub const SUB_MARK: OpIdx = 137;
    pub const SUB_END: OpIdx = 138;
    pub const EXPR_MARK: OpIdx = 139;
    pub const NOP: OpIdx = 140;
}

/// cc 值 → 比较算 op（`cmp_op_of` 的表形；越界归 !=）。
static CMP_OPS: [OpIdx; 6] = [
    ix::LT, ix::GT, ix::LE, ix::GE, ix::EQ, ix::NE,
];

/// The table — one row per variant, [`ix`] order (= enum declaration order).
pub static OPS: [OpSpec; 141] = [
    OpSpec {
        name: "PUSH_CONST",
        args: &[a(Opnd::Const16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_CONST_WIDE",
        args: &[Arg { pre: "[", post: "]", kind: Opnd::Const32 }],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_EMPTY",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_INT",
        args: &[a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_FLOAT",
        args: &[a(Opnd::F64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_TRUE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_FALSE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "POP",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "DUP",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LOAD_VAR",
        args: &[a(Opnd::Const32)],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STORE_VAR",
        args: &[a(Opnd::Const32)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STORE_VAR_POP",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LOAD_LOCAL",
        args: &[a(Opnd::Slot32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STORE_LOCAL",
        args: &[a(Opnd::Slot32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STORE_LOCAL_POP",
        args: &[a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::STORE_LOCAL), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "LOAD_ARRAY_ELEM",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STORE_ARRAY_ELEM",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "INCR_VAR",
        args: &[a(Opnd::Const32), a(Opnd::Imm64)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "INCR_VAR_POP",
        args: &[a(Opnd::Const16), a(Opnd::Imm64)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::INCR_VAR), args: &[LSel::F(0), LSel::F(1)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "INCR_LOCAL",
        args: &[a(Opnd::Slot32), a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "INCR_LOCAL_POP",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::INCR_LOCAL), args: &[LSel::F(0), LSel::F(1)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "APPEND_VAR",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "UNSET_VAR",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "VAR_EXISTS",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "PUSH_FRAME",
        args: &[a(Opnd::Count16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "POP_FRAME",
        args: &[],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "UPVAR",
        args: &[p(Opnd::Level16, "level="), p(Opnd::Const16, "src="), p(Opnd::Slot16, "dst=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "GLOBAL",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "JUMP",
        args: &[a(Opnd::Target32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "JUMP_TRUE",
        args: &[a(Opnd::Target32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "JUMP_FALSE",
        args: &[a(Opnd::Target32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LOOP_ENTER",
        args: &[p(Opnd::Target32, "cont="), p(Opnd::Target32, "brk=")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LOOP_EXIT",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "FOREACH_START",
        args: &[p(Opnd::Info32, "info="), p(Opnd::Target32, "next="), p(Opnd::Target32, "end=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "FOREACH_NEXT",
        args: &[p(Opnd::Info32, "info="), p(Opnd::Target32, "body=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "FOREACH_COLLECT",
        args: &[],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "FOREACH_END",
        args: &[],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BREAK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CONTINUE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "RETURN",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "RETURN_CODE",
        args: &[a(Opnd::Code32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EXIT",
        args: &[a(Opnd::Code32)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "ADD",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SUB",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LAPPEND_L",
        args: &[a(Opnd::Slot32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LAPPEND_V",
        args: &[a(Opnd::Const32)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LAPPEND_VV",
        args: &[a(Opnd::Const16), a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LAPPEND_V), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "LAPPEND_SS",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_LOCAL), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LAPPEND_L), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "BODY_MARK_FRAMED",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "ADD_SS",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "ADD_SI",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CMP_SS",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16), p(Opnd::Cc8, "cc=")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CMP_SI",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64), p(Opnd::Cc8, "cc=")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "ADD_VV",
        args: &[a(Opnd::Const16), a(Opnd::Const16)],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::ADD), args: &[] }],
    },
    OpSpec {
        name: "ADD_VI",
        args: &[a(Opnd::Const16), a(Opnd::Imm64)],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_INT), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::ADD), args: &[] }],
    },
    OpSpec {
        name: "CMP_VV",
        args: &[a(Opnd::Const16), a(Opnd::Const16), p(Opnd::Cc8, "cc=")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }],
    },
    OpSpec {
        name: "CMP_VI",
        args: &[a(Opnd::Const16), a(Opnd::Imm64), p(Opnd::Cc8, "cc=")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_INT), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }],
    },
    OpSpec {
        name: "CMP_SS_JT",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::CMP_SS), args: &[LSel::F(0), LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_TRUE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_SS_JF",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::CMP_SS), args: &[LSel::F(0), LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_FALSE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_SI_JT",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::CMP_SI), args: &[LSel::F(0), LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_TRUE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_SI_JF",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::CMP_SI), args: &[LSel::F(0), LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_FALSE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_VV_JT",
        args: &[a(Opnd::Const16), a(Opnd::Const16), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_TRUE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_VV_JF",
        args: &[a(Opnd::Const16), a(Opnd::Const16), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_FALSE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_VI_JT",
        args: &[a(Opnd::Const16), a(Opnd::Imm64), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_INT), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_TRUE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "CMP_VI_JF",
        args: &[a(Opnd::Const16), a(Opnd::Imm64), p(Opnd::Cc8, "cc="), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_INT), args: &[LSel::F(1)] }, Lower { op: OpSel::CmpOf(2), args: &[LSel::F(2)] }, Lower { op: OpSel::Idx(ix::JUMP_FALSE), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "MUL",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "DIV",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "MOD",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "POW",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "NEG",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EQ",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "NE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LT",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "GT",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "GE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STR_EQ",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STR_NE",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "AND",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "OR",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "NOT",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BIT_AND",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BIT_OR",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BIT_XOR",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BIT_NOT",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SHL",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SHR",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CONCAT",
        args: &[a(Opnd::Count16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "MAKE_LIST",
        args: &[a(Opnd::Count16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LIST_APPEND",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LIST_INDEX",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LIST_LENGTH",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STR_LEN",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "STR_INDEX",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EXPAND_LIST",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EXPAND_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CALL",
        args: &[p(Opnd::Count16, "cmd="), p(Opnd::Count32, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CALL_EXPAND",
        args: &[p(Opnd::Count16, "cmd="), p(Opnd::Count32, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "DYNCALL",
        args: &[p(Opnd::Count32, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "DYNCALL_EXPAND",
        args: &[p(Opnd::Count32, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CALL_PROC",
        args: &[p(Opnd::Count16, "proc="), p(Opnd::Count16, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "TAIL_CALL_PROC",
        args: &[p(Opnd::Count16, "proc="), p(Opnd::Count16, "argc=")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EVAL_SCRIPT",
        args: &[],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EVAL_EXPR",
        args: &[],
        top: OpClass::FlushOnly,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CATCH_START",
        args: &[a(Opnd::Target32)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "CATCH_END",
        args: &[],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "LINE",
        args: &[a(Opnd::Note32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BEGIN_CMD",
        args: &[a(Opnd::Site32)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "BEGIN_CMD_SUB",
        args: &[a(Opnd::Site32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::SUB_MARK), args: &[] }],
    },
    OpSpec {
        name: "EXPR_MARK_BEGIN",
        args: &[a(Opnd::Site32)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::EXPR_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "BEGIN_CMD_PUSH_CONST",
        args: &[a(Opnd::Site32), a(Opnd::Const16)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_CONST), args: &[LSel::F(1)] }],
    },
    OpSpec {
        name: "SUB_END_STORE_L_POP",
        args: &[a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::STORE_LOCAL), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "SUB_END_STORE_V_POP",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::STORE_VAR_POP), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "SUB_END_RETURN",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::RETURN), args: &[] }],
    },
    OpSpec {
        name: "SWAP_TO_NEXT_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::NOP), args: &[] }, Lower { op: OpSel::Idx(ix::NEXT_MARK), args: &[] }],
    },
    OpSpec {
        name: "NEXT_MARK_BEGIN_CMD",
        args: &[a(Opnd::Site32)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::NEXT_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "BEGIN_CMD_INCR_L_POP",
        args: &[a(Opnd::Site32), a(Opnd::Slot16), a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::INCR_LOCAL), args: &[LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "BEGIN_CMD_INCR_V_POP",
        args: &[a(Opnd::Site32), a(Opnd::Const16), a(Opnd::Imm64)],
        top: OpClass::FlushInvalidate,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::INCR_VAR), args: &[LSel::F(1), LSel::F(2)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "FOR_NEXT_INCR_L",
        args: &[a(Opnd::Site32), a(Opnd::Slot16), a(Opnd::Imm32), p(Opnd::Target32, "-> ")],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::NOP), args: &[] }, Lower { op: OpSel::Idx(ix::NEXT_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::INCR_LOCAL), args: &[LSel::F(1), LSel::FI64(2)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }, Lower { op: OpSel::Idx(ix::JUMP), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "FOR_NEXT_INCR_V",
        args: &[a(Opnd::Site32), a(Opnd::Const16), a(Opnd::Imm32), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushInvalidate,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::NOP), args: &[] }, Lower { op: OpSel::Idx(ix::NEXT_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::INCR_VAR), args: &[LSel::F(1), LSel::FI64(2)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }, Lower { op: OpSel::Idx(ix::JUMP), args: &[LSel::F(3)] }],
    },
    OpSpec {
        name: "FOREACH_NEXT_CLOSE",
        args: &[a(Opnd::Info32), p(Opnd::Target32, "-> ")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::NOP), args: &[] }, Lower { op: OpSel::Idx(ix::FOREACH_NEXT), args: &[LSel::F(0), LSel::F(1)] }],
    },
    OpSpec {
        name: "SET_SS_ADD",
        args: &[a(Opnd::Site32), a(Opnd::Site32), a(Opnd::Slot16), a(Opnd::Slot16), p(Opnd::Slot16, "-> ")],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::SUB_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::ADD_SS), args: &[LSel::F(2), LSel::F(3)] }, Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::STORE_LOCAL), args: &[LSel::F(4)] }, Lower { op: OpSel::Idx(ix::POP), args: &[] }],
    },
    OpSpec {
        name: "SET_VV_ADD",
        args: &[a(Opnd::Site32), a(Opnd::Site32), a(Opnd::Const16), a(Opnd::Const16), p(Opnd::Const16, "-> ")],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::SUB_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(2)] }, Lower { op: OpSel::Idx(ix::LOAD_VAR), args: &[LSel::F(3)] }, Lower { op: OpSel::Idx(ix::ADD), args: &[] }, Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::STORE_VAR_POP), args: &[LSel::F(4)] }],
    },
    OpSpec {
        name: "PUSH_CONST_SUB",
        args: &[a(Opnd::Const16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::PUSH_CONST), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::SUB_MARK), args: &[] }],
    },
    OpSpec {
        name: "SUB_MARK_BEGIN",
        args: &[a(Opnd::Site32)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "EXPR_MARK_BEGIN_PUSH",
        args: &[a(Opnd::Site32), a(Opnd::Const16)],
        top: OpClass::Pure,
        site_open: true,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::EXPR_MARK), args: &[] }, Lower { op: OpSel::Idx(ix::BEGIN_CMD), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_CONST), args: &[LSel::F(1)] }],
    },
    OpSpec {
        name: "SUB_SI",
        args: &[a(Opnd::Slot16), a(Opnd::Imm64)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_LOCAL), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::PUSH_INT), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::SUB), args: &[] }],
    },
    OpSpec {
        name: "SUB_SS",
        args: &[a(Opnd::Slot16), a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::LOAD_LOCAL), args: &[LSel::F(0)] }, Lower { op: OpSel::Idx(ix::LOAD_LOCAL), args: &[LSel::F(1)] }, Lower { op: OpSel::Idx(ix::SUB), args: &[] }],
    },
    OpSpec {
        name: "SUB_END_DYNCALL",
        args: &[a(Opnd::Count16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::DYNCALL), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "SUB_END_CALL",
        args: &[a(Opnd::Count16), a(Opnd::Count16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::CALL), args: &[LSel::F(0), LSel::F(1)] }],
    },
    OpSpec {
        name: "SUB_END_ADD",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::ADD), args: &[] }],
    },
    OpSpec {
        name: "INCR_V_STACK",
        args: &[a(Opnd::Const32)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SUB_END_INCR_V_STACK",
        args: &[a(Opnd::Const16)],
        top: OpClass::FlushInvalidate,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::INCR_V_STACK), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "INCR_L_STACK",
        args: &[a(Opnd::Slot32)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SUB_END_INCR_L_STACK",
        args: &[a(Opnd::Slot16)],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: true,
        lowers: &[Lower { op: OpSel::Idx(ix::SUB_END), args: &[] }, Lower { op: OpSel::Idx(ix::INCR_L_STACK), args: &[LSel::F(0)] }],
    },
    OpSpec {
        name: "BODY_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "NEXT_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SUB_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "SUB_END",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "EXPR_MARK",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: true,
        fused: false,
        lowers: &[],
    },
    OpSpec {
        name: "NOP",
        args: &[],
        top: OpClass::Pure,
        site_open: false,
        emittable: false,
        fused: false,
        lowers: &[],
    },
];

pub const OP_COUNT: usize = OPS.len();

/// (dense index, payload fields as u64 — f64 rides `to_bits`).
/// Exhaustive: a variant without an arm fails to compile.
pub fn decompose(op: &OpCode) -> (OpIdx, [u64; 5]) {
    match op {
        OpCode::PushConst(a0) => (0, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::PushConstWide(a0) => (1, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::PushEmpty => (2, [0, 0, 0, 0, 0]),
        OpCode::PushInt(a0) => (3, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::PushFloat(a0) => (4, [(*a0).to_bits(), 0, 0, 0, 0]),
        OpCode::PushTrue => (5, [0, 0, 0, 0, 0]),
        OpCode::PushFalse => (6, [0, 0, 0, 0, 0]),
        OpCode::Pop => (7, [0, 0, 0, 0, 0]),
        OpCode::Dup => (8, [0, 0, 0, 0, 0]),
        OpCode::LoadVar(a0) => (9, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::StoreVar(a0) => (10, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::StoreVarPop(a0) => (11, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::LoadLocal(a0) => (12, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::StoreLocal(a0) => (13, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::StoreLocalPop(a0) => (14, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::LoadArrayElem(a0) => (15, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::StoreArrayElem(a0) => (16, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::IncrVar(a0, a1) => (17, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::IncrVarPop(a0, a1) => (18, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::IncrLocal(a0, a1) => (19, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::IncrLocalPop(a0, a1) => (20, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::AppendVar(a0) => (21, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::UnsetVar(a0) => (22, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::VarExists(a0) => (23, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::PushFrame(a0) => (24, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::PopFrame => (25, [0, 0, 0, 0, 0]),
        OpCode::UpVar { level, src, dst } => (26, [*level as u64, *src as u64, *dst as u64, 0, 0]),
        OpCode::Global(a0) => (27, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::Jump(a0) => (28, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::JumpTrue(a0) => (29, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::JumpFalse(a0) => (30, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::LoopEnter { cont, brk } => (31, [*cont as u64, *brk as u64, 0, 0, 0]),
        OpCode::LoopExit => (32, [0, 0, 0, 0, 0]),
        OpCode::ForeachStart { info, next, end } => (33, [*info as u64, *next as u64, *end as u64, 0, 0]),
        OpCode::ForeachNext { info, body } => (34, [*info as u64, *body as u64, 0, 0, 0]),
        OpCode::ForeachCollect => (35, [0, 0, 0, 0, 0]),
        OpCode::ForeachEnd => (36, [0, 0, 0, 0, 0]),
        OpCode::Break => (37, [0, 0, 0, 0, 0]),
        OpCode::Continue => (38, [0, 0, 0, 0, 0]),
        OpCode::Return => (39, [0, 0, 0, 0, 0]),
        OpCode::ReturnCode(a0) => (40, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::Exit(a0) => (41, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::Add => (42, [0, 0, 0, 0, 0]),
        OpCode::Sub => (43, [0, 0, 0, 0, 0]),
        OpCode::LappendLocal(a0) => (44, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::LappendVar(a0) => (45, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::LappendVarVar(a0, a1) => (46, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::LappendSlotSlot(a0, a1) => (47, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::BodyMarkFramed => (48, [0, 0, 0, 0, 0]),
        OpCode::AddSlotSlot(a0, a1) => (49, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::AddSlotImm(a0, a1) => (50, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::CmpSlotSlot(a0, a1, a2) => (51, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::CmpSlotImm(a0, a1, a2) => (52, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::AddVarVar(a0, a1) => (53, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::AddVarImm(a0, a1) => (54, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::CmpVarVar(a0, a1, a2) => (55, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::CmpVarImm(a0, a1, a2) => (56, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::CmpSlotSlotJumpTrue(a0, a1, a2, a3) => (57, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpSlotSlotJumpFalse(a0, a1, a2, a3) => (58, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpSlotImmJumpTrue(a0, a1, a2, a3) => (59, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpSlotImmJumpFalse(a0, a1, a2, a3) => (60, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpVarVarJumpTrue(a0, a1, a2, a3) => (61, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpVarVarJumpFalse(a0, a1, a2, a3) => (62, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpVarImmJumpTrue(a0, a1, a2, a3) => (63, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::CmpVarImmJumpFalse(a0, a1, a2, a3) => (64, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::Mul => (65, [0, 0, 0, 0, 0]),
        OpCode::Div => (66, [0, 0, 0, 0, 0]),
        OpCode::Mod => (67, [0, 0, 0, 0, 0]),
        OpCode::Pow => (68, [0, 0, 0, 0, 0]),
        OpCode::Neg => (69, [0, 0, 0, 0, 0]),
        OpCode::Eq => (70, [0, 0, 0, 0, 0]),
        OpCode::Ne => (71, [0, 0, 0, 0, 0]),
        OpCode::Lt => (72, [0, 0, 0, 0, 0]),
        OpCode::Gt => (73, [0, 0, 0, 0, 0]),
        OpCode::Le => (74, [0, 0, 0, 0, 0]),
        OpCode::Ge => (75, [0, 0, 0, 0, 0]),
        OpCode::StrEq => (76, [0, 0, 0, 0, 0]),
        OpCode::StrNe => (77, [0, 0, 0, 0, 0]),
        OpCode::And => (78, [0, 0, 0, 0, 0]),
        OpCode::Or => (79, [0, 0, 0, 0, 0]),
        OpCode::Not => (80, [0, 0, 0, 0, 0]),
        OpCode::BitAnd => (81, [0, 0, 0, 0, 0]),
        OpCode::BitOr => (82, [0, 0, 0, 0, 0]),
        OpCode::BitXor => (83, [0, 0, 0, 0, 0]),
        OpCode::BitNot => (84, [0, 0, 0, 0, 0]),
        OpCode::Shl => (85, [0, 0, 0, 0, 0]),
        OpCode::Shr => (86, [0, 0, 0, 0, 0]),
        OpCode::Concat(a0) => (87, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::MakeList(a0) => (88, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::ListAppend => (89, [0, 0, 0, 0, 0]),
        OpCode::ListIndex => (90, [0, 0, 0, 0, 0]),
        OpCode::ListLength => (91, [0, 0, 0, 0, 0]),
        OpCode::StrLen => (92, [0, 0, 0, 0, 0]),
        OpCode::StrIndex => (93, [0, 0, 0, 0, 0]),
        OpCode::ExpandList => (94, [0, 0, 0, 0, 0]),
        OpCode::ExpandMark => (95, [0, 0, 0, 0, 0]),
        OpCode::Call { cmd_id, argc } => (96, [*cmd_id as u64, *argc as u64, 0, 0, 0]),
        OpCode::CallExpand { cmd_id, argc } => (97, [*cmd_id as u64, *argc as u64, 0, 0, 0]),
        OpCode::DynCall { argc } => (98, [*argc as u64, 0, 0, 0, 0]),
        OpCode::DynCallExpand { argc } => (99, [*argc as u64, 0, 0, 0, 0]),
        OpCode::CallProc { proc_id, argc } => (100, [*proc_id as u64, *argc as u64, 0, 0, 0]),
        OpCode::TailCallProc { proc_id, argc } => (101, [*proc_id as u64, *argc as u64, 0, 0, 0]),
        OpCode::EvalScript => (102, [0, 0, 0, 0, 0]),
        OpCode::EvalExpr => (103, [0, 0, 0, 0, 0]),
        OpCode::CatchStart(a0) => (104, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::CatchEnd => (105, [0, 0, 0, 0, 0]),
        OpCode::Line(a0) => (106, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::BeginCmd(a0) => (107, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::BeginCmdSubMark(a0) => (108, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::ExprMarkBeginCmd(a0) => (109, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::BeginCmdPushConst(a0, a1) => (110, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::SubEndStoreLocalPop(a0) => (111, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubEndStoreVarPop(a0) => (112, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubEndReturn => (113, [0, 0, 0, 0, 0]),
        OpCode::SwapToNextMark => (114, [0, 0, 0, 0, 0]),
        OpCode::NextMarkBeginCmd(a0) => (115, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::BeginCmdIncrLocalPop(a0, a1, a2) => (116, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::BeginCmdIncrVarPop(a0, a1, a2) => (117, [*a0 as u64, *a1 as u64, *a2 as u64, 0, 0]),
        OpCode::ForNextIncrLocal(a0, a1, a2, a3) => (118, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::ForNextIncrVar(a0, a1, a2, a3) => (119, [*a0 as u64, *a1 as u64, *a2 as u64, *a3 as u64, 0]),
        OpCode::ForeachNextClose(a0, a1) => (120, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::SlotSetAddSS { set_site, expr_site, a, b, dst } => (121, [*set_site as u64, *expr_site as u64, *a as u64, *b as u64, *dst as u64]),
        OpCode::VarSetAddVV { set_site, expr_site, a, b, dst } => (122, [*set_site as u64, *expr_site as u64, *a as u64, *b as u64, *dst as u64]),
        OpCode::PushConstSubMark(a0) => (123, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubMarkBeginCmd(a0) => (124, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::ExprMarkBeginPushConst(a0, a1) => (125, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::SubSlotImm(a0, a1) => (126, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::SubSlotSlot(a0, a1) => (127, [*a0 as u64, *a1 as u64, 0, 0, 0]),
        OpCode::SubEndDynCall(a0) => (128, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubEndCall { cmd_id, argc } => (129, [*cmd_id as u64, *argc as u64, 0, 0, 0]),
        OpCode::SubEndAdd => (130, [0, 0, 0, 0, 0]),
        OpCode::IncrVarStack(a0) => (131, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubEndIncrVarStack(a0) => (132, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::IncrLocalStack(a0) => (133, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::SubEndIncrLocalStack(a0) => (134, [*a0 as u64, 0, 0, 0, 0]),
        OpCode::BodyMark => (135, [0, 0, 0, 0, 0]),
        OpCode::NextMark => (136, [0, 0, 0, 0, 0]),
        OpCode::SubMark => (137, [0, 0, 0, 0, 0]),
        OpCode::SubEnd => (138, [0, 0, 0, 0, 0]),
        OpCode::ExprMark => (139, [0, 0, 0, 0, 0]),
        OpCode::Nop => (140, [0, 0, 0, 0, 0]),
    }
}

/// [`decompose`]'s inverse — round-trip verified by the census test.
/// Exhaustive: a variant without an arm fails to compile.
pub fn compose(idx: OpIdx, f: &[u64; 5]) -> Option<OpCode> {
    let g = |k: usize| f.get(k).copied().unwrap_or(0);
    Some(match idx {
        0 => OpCode::PushConst(g(0) as u16),
        1 => OpCode::PushConstWide(g(0) as u32),
        2 => OpCode::PushEmpty,
        3 => OpCode::PushInt(g(0) as i64),
        4 => OpCode::PushFloat(f64::from_bits(g(0))),
        5 => OpCode::PushTrue,
        6 => OpCode::PushFalse,
        7 => OpCode::Pop,
        8 => OpCode::Dup,
        9 => OpCode::LoadVar(g(0) as u32),
        10 => OpCode::StoreVar(g(0) as u32),
        11 => OpCode::StoreVarPop(g(0) as u16),
        12 => OpCode::LoadLocal(g(0) as u32),
        13 => OpCode::StoreLocal(g(0) as u32),
        14 => OpCode::StoreLocalPop(g(0) as u16),
        15 => OpCode::LoadArrayElem(g(0) as u16),
        16 => OpCode::StoreArrayElem(g(0) as u16),
        17 => OpCode::IncrVar(g(0) as u32, g(1) as i64),
        18 => OpCode::IncrVarPop(g(0) as u16, g(1) as i64),
        19 => OpCode::IncrLocal(g(0) as u32, g(1) as i64),
        20 => OpCode::IncrLocalPop(g(0) as u16, g(1) as i64),
        21 => OpCode::AppendVar(g(0) as u16),
        22 => OpCode::UnsetVar(g(0) as u16),
        23 => OpCode::VarExists(g(0) as u16),
        24 => OpCode::PushFrame(g(0) as u16),
        25 => OpCode::PopFrame,
        26 => OpCode::UpVar { level: g(0) as u16, src: g(1) as u16, dst: g(2) as u16 },
        27 => OpCode::Global(g(0) as u16),
        28 => OpCode::Jump(g(0) as u32),
        29 => OpCode::JumpTrue(g(0) as u32),
        30 => OpCode::JumpFalse(g(0) as u32),
        31 => OpCode::LoopEnter { cont: g(0) as u32, brk: g(1) as u32 },
        32 => OpCode::LoopExit,
        33 => OpCode::ForeachStart { info: g(0) as u32, next: g(1) as u32, end: g(2) as u32 },
        34 => OpCode::ForeachNext { info: g(0) as u32, body: g(1) as u32 },
        35 => OpCode::ForeachCollect,
        36 => OpCode::ForeachEnd,
        37 => OpCode::Break,
        38 => OpCode::Continue,
        39 => OpCode::Return,
        40 => OpCode::ReturnCode(g(0) as i32),
        41 => OpCode::Exit(g(0) as i32),
        42 => OpCode::Add,
        43 => OpCode::Sub,
        44 => OpCode::LappendLocal(g(0) as u32),
        45 => OpCode::LappendVar(g(0) as u32),
        46 => OpCode::LappendVarVar(g(0) as u16, g(1) as u16),
        47 => OpCode::LappendSlotSlot(g(0) as u16, g(1) as u16),
        48 => OpCode::BodyMarkFramed,
        49 => OpCode::AddSlotSlot(g(0) as u16, g(1) as u16),
        50 => OpCode::AddSlotImm(g(0) as u16, g(1) as i64),
        51 => OpCode::CmpSlotSlot(g(0) as u16, g(1) as u16, g(2) as u8),
        52 => OpCode::CmpSlotImm(g(0) as u16, g(1) as i64, g(2) as u8),
        53 => OpCode::AddVarVar(g(0) as u16, g(1) as u16),
        54 => OpCode::AddVarImm(g(0) as u16, g(1) as i64),
        55 => OpCode::CmpVarVar(g(0) as u16, g(1) as u16, g(2) as u8),
        56 => OpCode::CmpVarImm(g(0) as u16, g(1) as i64, g(2) as u8),
        57 => OpCode::CmpSlotSlotJumpTrue(g(0) as u16, g(1) as u16, g(2) as u8, g(3) as u32),
        58 => OpCode::CmpSlotSlotJumpFalse(g(0) as u16, g(1) as u16, g(2) as u8, g(3) as u32),
        59 => OpCode::CmpSlotImmJumpTrue(g(0) as u16, g(1) as i64, g(2) as u8, g(3) as u32),
        60 => OpCode::CmpSlotImmJumpFalse(g(0) as u16, g(1) as i64, g(2) as u8, g(3) as u32),
        61 => OpCode::CmpVarVarJumpTrue(g(0) as u16, g(1) as u16, g(2) as u8, g(3) as u32),
        62 => OpCode::CmpVarVarJumpFalse(g(0) as u16, g(1) as u16, g(2) as u8, g(3) as u32),
        63 => OpCode::CmpVarImmJumpTrue(g(0) as u16, g(1) as i64, g(2) as u8, g(3) as u32),
        64 => OpCode::CmpVarImmJumpFalse(g(0) as u16, g(1) as i64, g(2) as u8, g(3) as u32),
        65 => OpCode::Mul,
        66 => OpCode::Div,
        67 => OpCode::Mod,
        68 => OpCode::Pow,
        69 => OpCode::Neg,
        70 => OpCode::Eq,
        71 => OpCode::Ne,
        72 => OpCode::Lt,
        73 => OpCode::Gt,
        74 => OpCode::Le,
        75 => OpCode::Ge,
        76 => OpCode::StrEq,
        77 => OpCode::StrNe,
        78 => OpCode::And,
        79 => OpCode::Or,
        80 => OpCode::Not,
        81 => OpCode::BitAnd,
        82 => OpCode::BitOr,
        83 => OpCode::BitXor,
        84 => OpCode::BitNot,
        85 => OpCode::Shl,
        86 => OpCode::Shr,
        87 => OpCode::Concat(g(0) as u16),
        88 => OpCode::MakeList(g(0) as u16),
        89 => OpCode::ListAppend,
        90 => OpCode::ListIndex,
        91 => OpCode::ListLength,
        92 => OpCode::StrLen,
        93 => OpCode::StrIndex,
        94 => OpCode::ExpandList,
        95 => OpCode::ExpandMark,
        96 => OpCode::Call { cmd_id: g(0) as u16, argc: g(1) as u32 },
        97 => OpCode::CallExpand { cmd_id: g(0) as u16, argc: g(1) as u32 },
        98 => OpCode::DynCall { argc: g(0) as u32 },
        99 => OpCode::DynCallExpand { argc: g(0) as u32 },
        100 => OpCode::CallProc { proc_id: g(0) as u16, argc: g(1) as u16 },
        101 => OpCode::TailCallProc { proc_id: g(0) as u16, argc: g(1) as u16 },
        102 => OpCode::EvalScript,
        103 => OpCode::EvalExpr,
        104 => OpCode::CatchStart(g(0) as u32),
        105 => OpCode::CatchEnd,
        106 => OpCode::Line(g(0) as u32),
        107 => OpCode::BeginCmd(g(0) as u32),
        108 => OpCode::BeginCmdSubMark(g(0) as u32),
        109 => OpCode::ExprMarkBeginCmd(g(0) as u32),
        110 => OpCode::BeginCmdPushConst(g(0) as u32, g(1) as u16),
        111 => OpCode::SubEndStoreLocalPop(g(0) as u16),
        112 => OpCode::SubEndStoreVarPop(g(0) as u16),
        113 => OpCode::SubEndReturn,
        114 => OpCode::SwapToNextMark,
        115 => OpCode::NextMarkBeginCmd(g(0) as u32),
        116 => OpCode::BeginCmdIncrLocalPop(g(0) as u32, g(1) as u16, g(2) as i64),
        117 => OpCode::BeginCmdIncrVarPop(g(0) as u32, g(1) as u16, g(2) as i64),
        118 => OpCode::ForNextIncrLocal(g(0) as u32, g(1) as u16, g(2) as i32, g(3) as u32),
        119 => OpCode::ForNextIncrVar(g(0) as u32, g(1) as u16, g(2) as i32, g(3) as u32),
        120 => OpCode::ForeachNextClose(g(0) as u32, g(1) as u32),
        121 => OpCode::SlotSetAddSS { set_site: g(0) as u32, expr_site: g(1) as u32, a: g(2) as u16, b: g(3) as u16, dst: g(4) as u16 },
        122 => OpCode::VarSetAddVV { set_site: g(0) as u32, expr_site: g(1) as u32, a: g(2) as u16, b: g(3) as u16, dst: g(4) as u16 },
        123 => OpCode::PushConstSubMark(g(0) as u16),
        124 => OpCode::SubMarkBeginCmd(g(0) as u32),
        125 => OpCode::ExprMarkBeginPushConst(g(0) as u32, g(1) as u16),
        126 => OpCode::SubSlotImm(g(0) as u16, g(1) as i64),
        127 => OpCode::SubSlotSlot(g(0) as u16, g(1) as u16),
        128 => OpCode::SubEndDynCall(g(0) as u16),
        129 => OpCode::SubEndCall { cmd_id: g(0) as u16, argc: g(1) as u16 },
        130 => OpCode::SubEndAdd,
        131 => OpCode::IncrVarStack(g(0) as u32),
        132 => OpCode::SubEndIncrVarStack(g(0) as u16),
        133 => OpCode::IncrLocalStack(g(0) as u32),
        134 => OpCode::SubEndIncrLocalStack(g(0) as u16),
        135 => OpCode::BodyMark,
        136 => OpCode::NextMark,
        137 => OpCode::SubMark,
        138 => OpCode::SubEnd,
        139 => OpCode::ExprMark,
        140 => OpCode::Nop,
        _ => return None,
    })
}

/// The row for `op`.
pub fn spec_of(op: &OpCode) -> &'static OpSpec {
    let (idx, _) = decompose(op);
    &OPS[idx as usize]
}

/// Expand one fused op into its canonical window (the old `expand_into`);
/// `false` = not fused (copy verbatim).
pub fn expand_into(op: &OpCode, out: &mut Vec<OpCode>) -> bool {
    let (idx, f) = decompose(op);
    let spec = &OPS[idx as usize];
    if spec.lowers.is_empty() {
        return false;
    }
    let pick = |sel: &LSel| -> u64 {
        match *sel {
            LSel::F(k) => f[k as usize],
            LSel::FI64(k) => (f[k as usize] as u32 as i32) as i64 as u64,
            LSel::K16(v) => v as u64,
            LSel::K64(v) => v as u64,
        }
    };
    for low in spec.lowers {
        let mut nf = [0u64; 5];
        for (k, sel) in low.args.iter().enumerate() {
            nf[k] = pick(sel);
        }
        let target = match low.op {
            OpSel::Idx(i) => i,
            OpSel::CmpOf(k) => CMP_OPS[(f[k as usize] as usize).min(5)],
        };
        out.push(compose(target, &nf).expect("OPS lowers target is a valid index"));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcode::OpCode;

    /// One representative per variant, in `ix` order (the census the
    /// table invariant tests run over).
    pub fn census() -> Vec<OpCode> {
        vec![

            OpCode::PushConst(7),
            OpCode::PushConstWide(70001),
            OpCode::PushEmpty,
            OpCode::PushInt(-5),
            OpCode::PushFloat(1.5),
            OpCode::PushTrue,
            OpCode::PushFalse,
            OpCode::Pop,
            OpCode::Dup,
            OpCode::LoadVar(11),
            OpCode::StoreVar(12),
            OpCode::StoreVarPop(13),
            OpCode::LoadLocal(14),
            OpCode::StoreLocal(15),
            OpCode::StoreLocalPop(16),
            OpCode::LoadArrayElem(17),
            OpCode::StoreArrayElem(18),
            OpCode::IncrVar(19, -20),
            OpCode::IncrVarPop(21, 22),
            OpCode::IncrLocal(23, -24),
            OpCode::IncrLocalPop(25, 26),
            OpCode::AppendVar(27),
            OpCode::UnsetVar(28),
            OpCode::VarExists(29),
            OpCode::PushFrame(30),
            OpCode::PopFrame,
            OpCode::UpVar { level: 31, src: 32, dst: 33 },
            OpCode::Global(34),
            OpCode::Jump(35),
            OpCode::JumpTrue(36),
            OpCode::JumpFalse(37),
            OpCode::LoopEnter { cont: 38, brk: 39 },
            OpCode::LoopExit,
            OpCode::ForeachStart { info: 40, next: 41, end: 42 },
            OpCode::ForeachNext { info: 43, body: 44 },
            OpCode::ForeachCollect,
            OpCode::ForeachEnd,
            OpCode::Break,
            OpCode::Continue,
            OpCode::Return,
            OpCode::ReturnCode(45),
            OpCode::Exit(3),
            OpCode::Add,
            OpCode::Sub,
            OpCode::LappendLocal(46),
            OpCode::LappendVar(47),
            OpCode::LappendVarVar(48, 49),
            OpCode::LappendSlotSlot(50, 51),
            OpCode::BodyMarkFramed,
            OpCode::AddSlotSlot(52, 53),
            OpCode::AddSlotImm(54, -55),
            OpCode::CmpSlotSlot(56, 57, 2),
            OpCode::CmpSlotImm(58, -59, 4),
            OpCode::AddVarVar(60, 61),
            OpCode::AddVarImm(62, -63),
            OpCode::CmpVarVar(64, 65, 1),
            OpCode::CmpVarImm(66, -67, 5),
            OpCode::CmpSlotSlotJumpTrue(68, 69, 3, 70),
            OpCode::CmpSlotSlotJumpFalse(71, 72, 0, 73),
            OpCode::CmpSlotImmJumpTrue(74, -75, 2, 76),
            OpCode::CmpSlotImmJumpFalse(77, -78, 4, 79),
            OpCode::CmpVarVarJumpTrue(80, 81, 1, 82),
            OpCode::CmpVarVarJumpFalse(83, 84, 5, 85),
            OpCode::CmpVarImmJumpTrue(86, -87, 0, 88),
            OpCode::CmpVarImmJumpFalse(89, -90, 3, 91),
            OpCode::Mul,
            OpCode::Div,
            OpCode::Mod,
            OpCode::Pow,
            OpCode::Neg,
            OpCode::Eq,
            OpCode::Ne,
            OpCode::Lt,
            OpCode::Gt,
            OpCode::Le,
            OpCode::Ge,
            OpCode::StrEq,
            OpCode::StrNe,
            OpCode::And,
            OpCode::Or,
            OpCode::Not,
            OpCode::BitAnd,
            OpCode::BitOr,
            OpCode::BitXor,
            OpCode::BitNot,
            OpCode::Shl,
            OpCode::Shr,
            OpCode::Concat(92),
            OpCode::MakeList(93),
            OpCode::ListAppend,
            OpCode::ListIndex,
            OpCode::ListLength,
            OpCode::StrLen,
            OpCode::StrIndex,
            OpCode::ExpandList,
            OpCode::ExpandMark,
            OpCode::Call { cmd_id: 94, argc: 95 },
            OpCode::CallExpand { cmd_id: 96, argc: 97 },
            OpCode::DynCall { argc: 98 },
            OpCode::DynCallExpand { argc: 99 },
            OpCode::CallProc { proc_id: 100, argc: 101 },
            OpCode::TailCallProc { proc_id: 102, argc: 103 },
            OpCode::EvalScript,
            OpCode::EvalExpr,
            OpCode::CatchStart(104),
            OpCode::CatchEnd,
            OpCode::Line(105),
            OpCode::BeginCmd(106),
            OpCode::BeginCmdSubMark(107),
            OpCode::ExprMarkBeginCmd(108),
            OpCode::BeginCmdPushConst(109, 110),
            OpCode::SubEndStoreLocalPop(111),
            OpCode::SubEndStoreVarPop(112),
            OpCode::SubEndReturn,
            OpCode::SwapToNextMark,
            OpCode::NextMarkBeginCmd(113),
            OpCode::BeginCmdIncrLocalPop(114, 115, -116),
            OpCode::BeginCmdIncrVarPop(117, 118, -119),
            OpCode::ForNextIncrLocal(120, 121, -122, 123),
            OpCode::ForNextIncrVar(124, 125, -126, 127),
            OpCode::ForeachNextClose(128, 129),
            OpCode::SlotSetAddSS { set_site: 130, expr_site: 131, a: 132, b: 133, dst: 134 },
            OpCode::VarSetAddVV { set_site: 135, expr_site: 136, a: 137, b: 138, dst: 139 },
            OpCode::PushConstSubMark(140),
            OpCode::SubMarkBeginCmd(141),
            OpCode::ExprMarkBeginPushConst(142, 143),
            OpCode::SubSlotImm(144, -145),
            OpCode::SubSlotSlot(146, 147),
            OpCode::SubEndDynCall(148),
            OpCode::SubEndCall { cmd_id: 149, argc: 150 },
            OpCode::SubEndAdd,
            OpCode::IncrVarStack(151),
            OpCode::SubEndIncrVarStack(152),
            OpCode::IncrLocalStack(153),
            OpCode::SubEndIncrLocalStack(154),
            OpCode::BodyMark,
            OpCode::NextMark,
            OpCode::SubMark,
            OpCode::SubEnd,
            OpCode::ExprMark,
            OpCode::Nop,
        
        ]
    }

    /// decompose/compose/OPS must agree three ways: every variant has a
    /// unique dense index inside the table, the row's field count matches
    /// the payload, and compose(decompose(op)) rebuilds the op exactly.
    #[test]
    fn census_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for op in census() {
            let (idx, fields) = decompose(&op);
            assert!((idx as usize) < OPS.len(), "index {} out of table", idx);
            assert!(seen.insert(idx), "duplicate index {}", idx);
            let spec = &OPS[idx as usize];
            assert!(!spec.name.is_empty());
            for k in 0..5 {
                if fields[k] != 0 {
                    assert!(
                        k < spec.args.len(),
                        "{}: field {} set but row declares {} args",
                        spec.name, k, spec.args.len()
                    );
                }
            }
            let back = compose(idx, &fields).expect("compose");
            assert_eq!(back, op, "compose(decompose(op)) != op");
        }
        assert_eq!(seen.len(), OPS.len(), "census must cover every row");
    }

    /// Byte-exact disassembly: the table-driven Display must reproduce the
    /// per-variant formatting the pre-table implementation printed
    /// (captured at the D2 baseline; any delta is a formatting regression).
    #[test]
    fn display_golden() {
        let golden: &[&str] = &[
            "PUSH_CONST 7",
            "PUSH_CONST_WIDE [70001]",
            "PUSH_EMPTY",
            "PUSH_INT -5",
            "PUSH_FLOAT 1.5",
            "PUSH_TRUE",
            "PUSH_FALSE",
            "POP",
            "DUP",
            "LOAD_VAR 11",
            "STORE_VAR 12",
            "STORE_VAR_POP 13",
            "LOAD_LOCAL 14",
            "STORE_LOCAL 15",
            "STORE_LOCAL_POP 16",
            "LOAD_ARRAY_ELEM 17",
            "STORE_ARRAY_ELEM 18",
            "INCR_VAR 19 -20",
            "INCR_VAR_POP 21 22",
            "INCR_LOCAL 23 -24",
            "INCR_LOCAL_POP 25 26",
            "APPEND_VAR 27",
            "UNSET_VAR 28",
            "VAR_EXISTS 29",
            "PUSH_FRAME 30",
            "POP_FRAME",
            "UPVAR level=31 src=32 dst=33",
            "GLOBAL 34",
            "JUMP 35",
            "JUMP_TRUE 36",
            "JUMP_FALSE 37",
            "LOOP_ENTER cont=38 brk=39",
            "LOOP_EXIT",
            "FOREACH_START info=40 next=41 end=42",
            "FOREACH_NEXT info=43 body=44",
            "FOREACH_COLLECT",
            "FOREACH_END",
            "BREAK",
            "CONTINUE",
            "RETURN",
            "RETURN_CODE 45",
            "EXIT 3",
            "ADD",
            "SUB",
            "LAPPEND_L 46",
            "LAPPEND_V 47",
            "LAPPEND_VV 48 49",
            "LAPPEND_SS 50 51",
            "BODY_MARK_FRAMED",
            "ADD_SS 52 53",
            "ADD_SI 54 -55",
            "CMP_SS 56 57 cc=2",
            "CMP_SI 58 -59 cc=4",
            "ADD_VV 60 61",
            "ADD_VI 62 -63",
            "CMP_VV 64 65 cc=1",
            "CMP_VI 66 -67 cc=5",
            "CMP_SS_JT 68 69 cc=3 -> 70",
            "CMP_SS_JF 71 72 cc=0 -> 73",
            "CMP_SI_JT 74 -75 cc=2 -> 76",
            "CMP_SI_JF 77 -78 cc=4 -> 79",
            "CMP_VV_JT 80 81 cc=1 -> 82",
            "CMP_VV_JF 83 84 cc=5 -> 85",
            "CMP_VI_JT 86 -87 cc=0 -> 88",
            "CMP_VI_JF 89 -90 cc=3 -> 91",
            "MUL",
            "DIV",
            "MOD",
            "POW",
            "NEG",
            "EQ",
            "NE",
            "LT",
            "GT",
            "LE",
            "GE",
            "STR_EQ",
            "STR_NE",
            "AND",
            "OR",
            "NOT",
            "BIT_AND",
            "BIT_OR",
            "BIT_XOR",
            "BIT_NOT",
            "SHL",
            "SHR",
            "CONCAT 92",
            "MAKE_LIST 93",
            "LIST_APPEND",
            "LIST_INDEX",
            "LIST_LENGTH",
            "STR_LEN",
            "STR_INDEX",
            "EXPAND_LIST",
            "EXPAND_MARK",
            "CALL cmd=94 argc=95",
            "CALL_EXPAND cmd=96 argc=97",
            "DYNCALL argc=98",
            "DYNCALL_EXPAND argc=99",
            "CALL_PROC proc=100 argc=101",
            "TAIL_CALL_PROC proc=102 argc=103",
            "EVAL_SCRIPT",
            "EVAL_EXPR",
            "CATCH_START 104",
            "CATCH_END",
            "LINE 105",
            "BEGIN_CMD 106",
            "BEGIN_CMD_SUB 107",
            "EXPR_MARK_BEGIN 108",
            "BEGIN_CMD_PUSH_CONST 109 110",
            "SUB_END_STORE_L_POP 111",
            "SUB_END_STORE_V_POP 112",
            "SUB_END_RETURN",
            "SWAP_TO_NEXT_MARK",
            "NEXT_MARK_BEGIN_CMD 113",
            "BEGIN_CMD_INCR_L_POP 114 115 -116",
            "BEGIN_CMD_INCR_V_POP 117 118 -119",
            "FOR_NEXT_INCR_L 120 121 -122 -> 123",
            "FOR_NEXT_INCR_V 124 125 -126 -> 127",
            "FOREACH_NEXT_CLOSE 128 -> 129",
            "SET_SS_ADD 130 131 132 133 -> 134",
            "SET_VV_ADD 135 136 137 138 -> 139",
            "PUSH_CONST_SUB 140",
            "SUB_MARK_BEGIN 141",
            "EXPR_MARK_BEGIN_PUSH 142 143",
            "SUB_SI 144 -145",
            "SUB_SS 146 147",
            "SUB_END_DYNCALL 148",
            "SUB_END_CALL 149 150",
            "SUB_END_ADD",
            "INCR_V_STACK 151",
            "SUB_END_INCR_V_STACK 152",
            "INCR_L_STACK 153",
            "SUB_END_INCR_L_STACK 154",
            "BODY_MARK",
            "NEXT_MARK",
            "SUB_MARK",
            "SUB_END",
            "EXPR_MARK",
            "NOP",
        ];
        let census = census();
        assert_eq!(census.len(), golden.len());
        for (op, want) in census.iter().zip(golden) {
            assert_eq!(&op.to_string(), want, "Display drifted");
        }
    }
}
