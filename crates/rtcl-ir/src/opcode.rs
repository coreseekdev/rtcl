//! OpCode instruction set for the rtcl virtual machine.
//!
//! Instructions are organised into tiers:
//!
//! - **Primitive opcodes** — stack manipulation, variables, arithmetic,
//!   comparison, logical and bitwise operations, control flow (jumps, loops,
//!   return, break/continue).  These are executed directly by the VM dispatch
//!   loop with no interpreter callback.
//!
//! - **Call** — invoke a *known* command by numeric ID.  The command ID
//!   space is split into two ranges: `0..127` for standard library commands
//!   and `128..` for extension / platform-dependent commands.  The VM
//!   dispatches through a function-pointer table (no HashMap lookup).
//!
//! - **DynCall** — invoke a command whose name is on the stack.  Used when
//!   the command name cannot be resolved at compile time.
//!
//! ## Loop management
//!
//! Loops are bracketed by `LoopEnter` / `LoopExit`.  The VM maintains a loop
//! stack so that `Break` / `Continue` opcodes (or corresponding errors from
//! `EvalScript`) can be resolved to the correct jump targets.

use core::fmt;

// ---------------------------------------------------------------------------
// Command-ID enumeration — shared between compiler and VM
// ---------------------------------------------------------------------------

/// Unified command IDs for built-in commands dispatched via `Call`.
///
/// IDs `0..127` are standard library commands (always available).
/// IDs `128..` are extension / platform commands (may be gated by features).
///
/// **Do not reorder** existing entries without updating the interpreter's
/// registration code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CmdId {
    // === Standard library (0..127) ==========================================

    // -- Language commands (complex, still need interp callback) --------------
    Foreach   =  0,
    Switch    =  1,
    Try       =  2,
    Catch     =  3,
    Proc      =  4,
    Rename    =  5,
    Eval      =  6,
    Apply     =  7,
    Uplevel   =  8,
    Upvar     =  9,
    Global    = 10,
    Unset     = 11,
    Subst     = 12,
    Info      = 13,
    Error     = 14,
    Tailcall  = 15,
    Append    = 16,
    // -- Standard data-manipulation commands ----------------------------------
    StringCmd = 32,
    List      = 33,
    Llength   = 34,
    Lindex    = 35,
    Lappend   = 36,
    Lrange    = 37,
    Lsearch   = 38,
    Lsort     = 39,
    Linsert   = 40,
    Lreplace  = 41,
    Lassign   = 42,
    Lrepeat   = 43,
    Lreverse  = 44,
    Concat    = 45,
    Split     = 46,
    Join      = 47,
    Lmap      = 48,
    Lset      = 49,
    Dict      = 50,
    Array     = 51,
    Format    = 52,
    Scan      = 53,
    Range     = 54,
    Time      = 55,
    Timerate  = 56,

    // === Extension / platform (128..) =======================================
    Puts        = 128,
    Source      = 129,
    File        = 130,
    Glob        = 131,
    Regexp      = 132,
    Regsub      = 133,
    Disassemble = 134,
}

impl CmdId {
    /// Whether this ID refers to an extension / platform command (ID >= 128).
    pub fn is_extension(self) -> bool {
        (self as u16) >= 128
    }

    /// Try to map a command name to its numeric ID.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            // -- Standard library ---
            "foreach"     => Self::Foreach,
            "switch"      => Self::Switch,
            "try"         => Self::Try,
            "catch"       => Self::Catch,
            "proc"        => Self::Proc,
            "rename"      => Self::Rename,
            "eval"        => Self::Eval,
            "apply"       => Self::Apply,
            "uplevel"     => Self::Uplevel,
            "upvar"       => Self::Upvar,
            "global"      => Self::Global,
            "unset"       => Self::Unset,
            "subst"       => Self::Subst,
            "info"        => Self::Info,
            "error"       => Self::Error,
            "tailcall"    => Self::Tailcall,
            "append"      => Self::Append,
            "string"      => Self::StringCmd,
            "list"        => Self::List,
            "llength"     => Self::Llength,
            "lindex"      => Self::Lindex,
            "lappend"     => Self::Lappend,
            "lrange"      => Self::Lrange,
            "lsearch"     => Self::Lsearch,
            "lsort"       => Self::Lsort,
            "linsert"     => Self::Linsert,
            "lreplace"    => Self::Lreplace,
            "lassign"     => Self::Lassign,
            "lrepeat"     => Self::Lrepeat,
            "lreverse"    => Self::Lreverse,
            "concat"      => Self::Concat,
            "split"       => Self::Split,
            "join"        => Self::Join,
            "lmap"        => Self::Lmap,
            "lset"        => Self::Lset,
            "dict"        => Self::Dict,
            "array"       => Self::Array,
            "format"      => Self::Format,
            "scan"        => Self::Scan,
            "range"       => Self::Range,
            "time"        => Self::Time,
            "timerate"    => Self::Timerate,
            // -- Extension / platform ---
            "puts"        => Self::Puts,
            "source"      => Self::Source,
            "file"        => Self::File,
            "glob"        => Self::Glob,
            "regexp"      => Self::Regexp,
            "regsub"      => Self::Regsub,
            "disassemble" => Self::Disassemble,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// OpCode — the instruction set
// ---------------------------------------------------------------------------

/// A single VM instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    // ── Stack manipulation ──────────────────────────────────────────────────

    /// Push a constant from the constant pool.
    PushConst(u16),

    /// Push a constant from the constant pool (wide index for large pools).
    PushConstWide(u32),

    /// Push the empty string.
    PushEmpty,

    /// Push a small integer literal.
    PushInt(i64),

    /// Push a floating-point literal.
    PushFloat(f64),

    /// Push boolean `true` (integer 1).
    PushTrue,

    /// Push boolean `false` (integer 0).
    PushFalse,

    /// Pop and discard TOS.
    Pop,

    /// Duplicate TOS.
    Dup,

    // ── Variable access (namespace-aware) ───────────────────────────────────
    //
    // Name indices point into the constant pool.  At runtime the VM resolves
    // through the scope chain: local frame → upvar links → namespace → global.

    /// Load a variable by name (constant-pool index).
    /// Resolves through the current scope chain.
    LoadVar(u16),

    /// Store TOS into a variable by name.  Leaves the value on the stack.
    StoreVar(u16),

    /// Store TOS into a variable by name and **pop** the value (discard).
    /// Used when the result of `set` is not needed.
    StoreVarPop(u16),

    /// Load from a call-frame slot (local variable, fast path).
    LoadLocal(u16),

    /// Store TOS into a call-frame slot.  Leaves the value on the stack.
    StoreLocal(u16),

    /// Load an array element: `$name(TOS)`.  Name from constant pool.
    LoadArrayElem(u16),

    /// Store TOS-1 into `name(TOS)`.  Pops index, leaves value.
    StoreArrayElem(u16),

    /// Increment a variable by a signed immediate and push the new value.
    IncrVar(u16, i64),

    /// Append TOS to the variable (constant-pool name index).
    /// Pops the append value, pushes the new variable value.
    AppendVar(u16),

    /// Unset a variable by name (constant-pool index).
    UnsetVar(u16),

    /// Push `1` if the variable exists, `0` otherwise.
    VarExists(u16),

    // ── Scope / namespace ───────────────────────────────────────────────────

    /// Push a new call frame.  Operand = number of local slots to reserve.
    PushFrame(u16),

    /// Pop the current call frame (restores the enclosing scope).
    PopFrame,

    /// `upvar`: link the call-frame slot `dst` (u16) to the variable named
    /// by constant-pool index `src` (u16) in the frame `level` levels up.
    UpVar { level: u16, src: u16, dst: u16 },

    /// Declare a local name as referring to the global variable of the same
    /// name (constant-pool index).
    Global(u16),

    // ── Control flow ────────────────────────────────────────────────────────

    /// Unconditional jump (absolute instruction offset).
    Jump(u32),

    /// Jump if TOS is true (pops).
    JumpTrue(u32),

    /// Jump if TOS is false (pops).
    JumpFalse(u32),

    // ── Loop management ─────────────────────────────────────────────────────
    //
    // The VM maintains a loop stack.  `LoopEnter` pushes a descriptor;
    // `LoopExit` pops it.  When the VM encounters a `Break` / `Continue`
    // opcode (or catches the corresponding error from `EvalScript`), it
    // reads the top of the loop stack to find the correct jump target.

    /// Enter a loop context.
    /// `cont` = continue target (PC), `brk` = break target (PC).
    LoopEnter { cont: u32, brk: u32 },

    /// Leave a loop context.
    LoopExit,

    /// Break out of the innermost loop.
    /// If executing inside a direct bytecode loop, the VM jumps to the
    /// loop's `brk` target.  If no loop is active (e.g. break inside a
    /// dynamically `eval`-ed string), the opcode raises `Error::Break`.
    Break,

    /// Continue the innermost loop (analogous to `Break`).
    Continue,

    // ── Return / exit ───────────────────────────────────────────────────────

    /// Return from the current proc with TOS as the value.
    Return,

    /// Return with a specific `-code` value.
    ReturnCode(i32),

    /// Terminate the interpreter (exit code).
    Exit(i32),

    // ── Arithmetic ──────────────────────────────────────────────────────────

    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Neg,

    // ── Comparison ──────────────────────────────────────────────────────────

    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    StrEq,
    StrNe,

    // ── Logical ─────────────────────────────────────────────────────────────

    And,
    Or,
    Not,

    // ── Bitwise ─────────────────────────────────────────────────────────────

    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,

    // ── String / List primitives ────────────────────────────────────────────

    /// Concatenate the top `n` stack values into a string.
    Concat(u16),

    /// Build a Tcl list from the top `n` stack values.
    MakeList(u16),

    /// Append TOS to the list below TOS.
    ListAppend,

    /// Index into a list: `[list, index]` → `[element]`.
    ListIndex,

    /// Push the length of the list at TOS.
    ListLength,

    /// Push the string length of TOS.
    StrLen,

    /// String index: `[string, index]` → `[char]`.
    StrIndex,

    /// Expand TOS as a list — pushes individual elements.
    ExpandList,

    // ── Command calls ───────────────────────────────────────────────────────

    /// **Call** — invoke a built-in command by numeric [`CmdId`].
    ///
    /// `cmd_id` indexes the interpreter's unified command table.
    /// `argc` arguments (including the command name itself) are on the stack.
    Call { cmd_id: u16, argc: u16 },

    /// **CallExpand** — like `Call`, but the argument list may contain
    /// `{*}` expanded elements.  `argc` is the *compile-time* word count;
    /// the actual argument count is determined at runtime by tracking
    /// preceding `ExpandList` ops.
    CallExpand { cmd_id: u16, argc: u16 },

    /// **DynCall** — dynamic command invocation.
    ///
    /// The command name is the *first* of `argc` values on the stack.
    DynCall { argc: u16 },

    /// **DynCallExpand** — dynamic call with `{*}` expansion.
    DynCallExpand { argc: u16 },

    /// Call a user-defined proc by ID.
    CallProc { proc_id: u16, argc: u16 },

    /// Tail-call a user-defined proc — reuses the current call frame
    /// instead of pushing a new one.
    TailCallProc { proc_id: u16, argc: u16 },

    // ── Special ─────────────────────────────────────────────────────────────

    /// Evaluate TOS as a Tcl script.
    EvalScript,

    /// Evaluate TOS as a Tcl expression (like `expr {...}`).
    EvalExpr,

    /// Begin a `catch` block — operand is the error-handler PC.
    CatchStart(u32),

    /// End a `catch` block.
    CatchEnd,

    // ── Debug / meta ────────────────────────────────────────────────────────

    /// Source-line annotation (for error messages).
    Line(u32),

    /// No operation.
    Nop,
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Stack
            OpCode::PushConst(idx) => write!(f, "PUSH_CONST {}", idx),
            OpCode::PushConstWide(idx) => write!(f, "PUSH_CONST_WIDE [{}]", idx),
            OpCode::PushEmpty => write!(f, "PUSH_EMPTY"),
            OpCode::PushInt(n) => write!(f, "PUSH_INT {}", n),
            OpCode::PushFloat(n) => write!(f, "PUSH_FLOAT {}", n),
            OpCode::PushTrue => write!(f, "PUSH_TRUE"),
            OpCode::PushFalse => write!(f, "PUSH_FALSE"),
            OpCode::Pop => write!(f, "POP"),
            OpCode::Dup => write!(f, "DUP"),

            // Variables
            OpCode::LoadVar(idx) => write!(f, "LOAD_VAR {}", idx),
            OpCode::StoreVar(idx) => write!(f, "STORE_VAR {}", idx),
            OpCode::StoreVarPop(idx) => write!(f, "STORE_VAR_POP {}", idx),
            OpCode::LoadLocal(slot) => write!(f, "LOAD_LOCAL {}", slot),
            OpCode::StoreLocal(slot) => write!(f, "STORE_LOCAL {}", slot),
            OpCode::LoadArrayElem(idx) => write!(f, "LOAD_ARRAY_ELEM {}", idx),
            OpCode::StoreArrayElem(idx) => write!(f, "STORE_ARRAY_ELEM {}", idx),
            OpCode::IncrVar(idx, n) => write!(f, "INCR_VAR {} {}", idx, n),
            OpCode::AppendVar(idx) => write!(f, "APPEND_VAR {}", idx),
            OpCode::UnsetVar(idx) => write!(f, "UNSET_VAR {}", idx),
            OpCode::VarExists(idx) => write!(f, "VAR_EXISTS {}", idx),

            // Scope
            OpCode::PushFrame(n) => write!(f, "PUSH_FRAME {}", n),
            OpCode::PopFrame => write!(f, "POP_FRAME"),
            OpCode::UpVar { level, src, dst } => {
                write!(f, "UPVAR level={} src={} dst={}", level, src, dst)
            }
            OpCode::Global(idx) => write!(f, "GLOBAL {}", idx),

            // Control flow
            OpCode::Jump(off) => write!(f, "JUMP {}", off),
            OpCode::JumpTrue(off) => write!(f, "JUMP_TRUE {}", off),
            OpCode::JumpFalse(off) => write!(f, "JUMP_FALSE {}", off),

            // Loop management
            OpCode::LoopEnter { cont, brk } => {
                write!(f, "LOOP_ENTER cont={} brk={}", cont, brk)
            }
            OpCode::LoopExit => write!(f, "LOOP_EXIT"),
            OpCode::Break => write!(f, "BREAK"),
            OpCode::Continue => write!(f, "CONTINUE"),

            // Return / exit
            OpCode::Return => write!(f, "RETURN"),
            OpCode::ReturnCode(c) => write!(f, "RETURN_CODE {}", c),
            OpCode::Exit(c) => write!(f, "EXIT {}", c),

            // Arithmetic
            OpCode::Add => write!(f, "ADD"),
            OpCode::Sub => write!(f, "SUB"),
            OpCode::Mul => write!(f, "MUL"),
            OpCode::Div => write!(f, "DIV"),
            OpCode::Mod => write!(f, "MOD"),
            OpCode::Pow => write!(f, "POW"),
            OpCode::Neg => write!(f, "NEG"),

            // Comparison
            OpCode::Eq => write!(f, "EQ"),
            OpCode::Ne => write!(f, "NE"),
            OpCode::Lt => write!(f, "LT"),
            OpCode::Gt => write!(f, "GT"),
            OpCode::Le => write!(f, "LE"),
            OpCode::Ge => write!(f, "GE"),
            OpCode::StrEq => write!(f, "STR_EQ"),
            OpCode::StrNe => write!(f, "STR_NE"),

            // Logical
            OpCode::And => write!(f, "AND"),
            OpCode::Or => write!(f, "OR"),
            OpCode::Not => write!(f, "NOT"),

            // Bitwise
            OpCode::BitAnd => write!(f, "BIT_AND"),
            OpCode::BitOr => write!(f, "BIT_OR"),
            OpCode::BitXor => write!(f, "BIT_XOR"),
            OpCode::BitNot => write!(f, "BIT_NOT"),
            OpCode::Shl => write!(f, "SHL"),
            OpCode::Shr => write!(f, "SHR"),

            // String / List
            OpCode::Concat(n) => write!(f, "CONCAT {}", n),
            OpCode::MakeList(n) => write!(f, "MAKE_LIST {}", n),
            OpCode::ListAppend => write!(f, "LIST_APPEND"),
            OpCode::ListIndex => write!(f, "LIST_INDEX"),
            OpCode::ListLength => write!(f, "LIST_LENGTH"),
            OpCode::StrLen => write!(f, "STR_LEN"),
            OpCode::StrIndex => write!(f, "STR_INDEX"),
            OpCode::ExpandList => write!(f, "EXPAND_LIST"),

            // Command calls
            OpCode::Call { cmd_id, argc } => {
                write!(f, "CALL cmd={} argc={}", cmd_id, argc)
            }
            OpCode::CallExpand { cmd_id, argc } => {
                write!(f, "CALL_EXPAND cmd={} argc={}", cmd_id, argc)
            }
            OpCode::DynCall { argc } => write!(f, "DYNCALL argc={}", argc),
            OpCode::DynCallExpand { argc } => write!(f, "DYNCALL_EXPAND argc={}", argc),
            OpCode::CallProc { proc_id, argc } => {
                write!(f, "CALL_PROC proc={} argc={}", proc_id, argc)
            }
            OpCode::TailCallProc { proc_id, argc } => {
                write!(f, "TAIL_CALL_PROC proc={} argc={}", proc_id, argc)
            }

            // Special
            OpCode::EvalScript => write!(f, "EVAL_SCRIPT"),
            OpCode::EvalExpr => write!(f, "EVAL_EXPR"),
            OpCode::CatchStart(off) => write!(f, "CATCH_START {}", off),
            OpCode::CatchEnd => write!(f, "CATCH_END"),

            // Debug
            OpCode::Line(n) => write!(f, "LINE {}", n),
            OpCode::Nop => write!(f, "NOP"),
        }
    }
}
