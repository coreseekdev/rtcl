//! OpCode instruction set for the rtcl virtual machine.
//!
//! Each variant represents a single VM instruction.  Instructions operate on
//! an implicit operand stack.

use core::fmt;

/// A single VM instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    // -- Stack manipulation ---------------------------------------------------

    /// Push a constant from the constant pool onto the stack.
    /// Operand: index into [`ByteCode::constants`].
    PushConst(u16),

    /// Push the empty string.
    PushEmpty,

    /// Push an integer literal (small values that don't need the pool).
    PushInt(i64),

    /// Pop and discard the top of the stack.
    Pop,

    /// Duplicate the top of the stack.
    Dup,

    // -- Variables -------------------------------------------------------------

    /// Load a local variable (slot index).
    LoadLocal(u16),

    /// Store TOS into a local variable (slot index).
    StoreLocal(u16),

    /// Load a global variable by name (name index into constant pool).
    LoadGlobal(u16),

    /// Store TOS into a global variable by name (name index).
    StoreGlobal(u16),

    /// Load an array element: `$name(index)`.
    /// Stack: `[name_idx, index]` → `[value]`
    LoadArray(u16),

    /// Store TOS into array element.
    /// Stack: `[value, name_idx, index]` → `[]`
    StoreArray(u16),

    /// Unset a variable (name index).
    UnsetVar(u16),

    // -- Command invocation ---------------------------------------------------

    /// Invoke a built-in command by id with `argc` arguments on the stack.
    /// Stack: `[arg0 .. argN]` → `[result]`
    InvokeBuiltin { cmd_id: u16, argc: u16 },

    /// Invoke a user-defined proc by id.
    /// Stack: `[arg0 .. argN]` → `[result]`
    InvokeProc { proc_id: u16, argc: u16 },

    /// Dynamic command invocation — command name is TOS.
    /// Stack: `[name, arg0 .. argN]` → `[result]`
    InvokeDynamic { argc: u16 },

    // -- Control flow ---------------------------------------------------------

    /// Unconditional jump (absolute instruction offset).
    Jump(u32),

    /// Jump if TOS is true (pops TOS).
    JumpTrue(u32),

    /// Jump if TOS is false (pops TOS).
    JumpFalse(u32),

    /// Return from the current proc with TOS as the result.
    Return,

    /// Signal a `break` from within a loop.
    Break,

    /// Signal a `continue` within a loop.
    Continue,

    /// Enter a new variable scope (proc call).
    EnterScope,

    /// Leave the current variable scope.
    LeaveScope,

    // -- Arithmetic -----------------------------------------------------------

    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Neg,

    // -- Comparison -----------------------------------------------------------

    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    StrEq,
    StrNe,

    // -- Logical --------------------------------------------------------------

    And,
    Or,
    Not,

    // -- Bitwise --------------------------------------------------------------

    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,

    // -- String / List --------------------------------------------------------

    /// Concatenate the top `n` stack values into one string.
    Concat(u16),

    /// Build a Tcl list from the top `n` stack values.
    MakeList(u16),

    /// Append TOS to the list below TOS.
    ListAppend,

    /// Index into a list: `[list, index]` → `[element]`.
    ListIndex,

    /// String length of TOS.
    StrLen,

    /// String index: `[string, index]` → `[char]`.
    StrIndex,

    // -- Special --------------------------------------------------------------

    /// Evaluate an arbitrary script string (TOS). Used for dynamic `eval`.
    EvalScript,

    /// Evaluate TOS as a Tcl expression (like `expr`). Returns the result.
    EvalExpr,

    /// Begin a `catch` block — the operand is the jump target for errors.
    CatchStart(u32),

    /// End a `catch` block.
    CatchEnd,

    /// Source line annotation (for error reporting).
    Line(u32),

    /// Expand TOS as a list — pushes `n` separate values + the count.
    ExpandList,

    /// No operation.
    Nop,
}

impl fmt::Display for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpCode::PushConst(idx) => write!(f, "PUSH_CONST {}", idx),
            OpCode::PushEmpty => write!(f, "PUSH_EMPTY"),
            OpCode::PushInt(n) => write!(f, "PUSH_INT {}", n),
            OpCode::Pop => write!(f, "POP"),
            OpCode::Dup => write!(f, "DUP"),
            OpCode::LoadLocal(slot) => write!(f, "LOAD_LOCAL {}", slot),
            OpCode::StoreLocal(slot) => write!(f, "STORE_LOCAL {}", slot),
            OpCode::LoadGlobal(idx) => write!(f, "LOAD_GLOBAL {}", idx),
            OpCode::StoreGlobal(idx) => write!(f, "STORE_GLOBAL {}", idx),
            OpCode::LoadArray(idx) => write!(f, "LOAD_ARRAY {}", idx),
            OpCode::StoreArray(idx) => write!(f, "STORE_ARRAY {}", idx),
            OpCode::UnsetVar(idx) => write!(f, "UNSET_VAR {}", idx),
            OpCode::InvokeBuiltin { cmd_id, argc } => {
                write!(f, "INVOKE_BUILTIN cmd={} argc={}", cmd_id, argc)
            }
            OpCode::InvokeProc { proc_id, argc } => {
                write!(f, "INVOKE_PROC proc={} argc={}", proc_id, argc)
            }
            OpCode::InvokeDynamic { argc } => write!(f, "INVOKE_DYNAMIC argc={}", argc),
            OpCode::Jump(off) => write!(f, "JUMP {}", off),
            OpCode::JumpTrue(off) => write!(f, "JUMP_TRUE {}", off),
            OpCode::JumpFalse(off) => write!(f, "JUMP_FALSE {}", off),
            OpCode::Return => write!(f, "RETURN"),
            OpCode::Break => write!(f, "BREAK"),
            OpCode::Continue => write!(f, "CONTINUE"),
            OpCode::EnterScope => write!(f, "ENTER_SCOPE"),
            OpCode::LeaveScope => write!(f, "LEAVE_SCOPE"),
            OpCode::Add => write!(f, "ADD"),
            OpCode::Sub => write!(f, "SUB"),
            OpCode::Mul => write!(f, "MUL"),
            OpCode::Div => write!(f, "DIV"),
            OpCode::Mod => write!(f, "MOD"),
            OpCode::Pow => write!(f, "POW"),
            OpCode::Neg => write!(f, "NEG"),
            OpCode::Eq => write!(f, "EQ"),
            OpCode::Ne => write!(f, "NE"),
            OpCode::Lt => write!(f, "LT"),
            OpCode::Gt => write!(f, "GT"),
            OpCode::Le => write!(f, "LE"),
            OpCode::Ge => write!(f, "GE"),
            OpCode::StrEq => write!(f, "STR_EQ"),
            OpCode::StrNe => write!(f, "STR_NE"),
            OpCode::And => write!(f, "AND"),
            OpCode::Or => write!(f, "OR"),
            OpCode::Not => write!(f, "NOT"),
            OpCode::BitAnd => write!(f, "BIT_AND"),
            OpCode::BitOr => write!(f, "BIT_OR"),
            OpCode::BitXor => write!(f, "BIT_XOR"),
            OpCode::BitNot => write!(f, "BIT_NOT"),
            OpCode::Shl => write!(f, "SHL"),
            OpCode::Shr => write!(f, "SHR"),
            OpCode::Concat(n) => write!(f, "CONCAT {}", n),
            OpCode::MakeList(n) => write!(f, "MAKE_LIST {}", n),
            OpCode::ListAppend => write!(f, "LIST_APPEND"),
            OpCode::ListIndex => write!(f, "LIST_INDEX"),
            OpCode::StrLen => write!(f, "STR_LEN"),
            OpCode::StrIndex => write!(f, "STR_INDEX"),
            OpCode::EvalScript => write!(f, "EVAL_SCRIPT"),
            OpCode::EvalExpr => write!(f, "EVAL_EXPR"),
            OpCode::CatchStart(off) => write!(f, "CATCH_START {}", off),
            OpCode::CatchEnd => write!(f, "CATCH_END"),
            OpCode::Line(n) => write!(f, "LINE {}", n),
            OpCode::ExpandList => write!(f, "EXPAND_LIST"),
            OpCode::Nop => write!(f, "NOP"),
        }
    }
}
