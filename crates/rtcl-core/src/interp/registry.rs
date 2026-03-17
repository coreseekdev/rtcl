//! Command registry — single-source command table, built-in registration, and
//! CmdId dispatch.

use super::Interp;
use crate::command::{CommandFunc, CommandCategory};
use crate::error::{Error, Result};
use rtcl_parser::CmdId;

use super::commands::*;
use CommandCategory::*;

/// A row in the master command table.
struct CmdEntry {
    name: &'static str,
    func: CommandFunc,
    cat: CommandCategory,
    /// `Some(id)` when the compiler can emit `Call(CmdId)` for this command.
    cmd_id: Option<u16>,
}

/// Master command table — single source of truth for built-in commands.
/// Both `register_builtins()` and `resolve_cmd()` are derived from this table.
/// Commands that use native VM opcodes (set, if, while, for, break, continue,
/// return, exit, expr, incr) have `cmd_id: None`.
static CMD_TABLE: &[CmdEntry] = &[
    // ── Language builtins ──────────────────────────────────────────────────
    CmdEntry { name: "set",       func: misc::cmd_set,          cat: Language, cmd_id: None },
    CmdEntry { name: "if",        func: control::cmd_if,        cat: Language, cmd_id: None },
    CmdEntry { name: "while",     func: loops::cmd_while,       cat: Language, cmd_id: None },
    CmdEntry { name: "for",       func: loops::cmd_for,         cat: Language, cmd_id: None },
    CmdEntry { name: "break",     func: control::cmd_break,     cat: Language, cmd_id: None },
    CmdEntry { name: "continue",  func: control::cmd_continue,  cat: Language, cmd_id: None },
    CmdEntry { name: "return",    func: control::cmd_return,    cat: Language, cmd_id: None },
    CmdEntry { name: "exit",      func: control::cmd_exit,      cat: Language, cmd_id: None },
    CmdEntry { name: "expr",      func: misc::cmd_expr,         cat: Language, cmd_id: None },
    CmdEntry { name: "incr",      func: misc::cmd_incr,         cat: Language, cmd_id: None },
    CmdEntry { name: "foreach",   func: loops::cmd_foreach,     cat: Language, cmd_id: Some(CmdId::Foreach  as u16) },
    CmdEntry { name: "switch",    func: control::cmd_switch,    cat: Language, cmd_id: Some(CmdId::Switch   as u16) },
    CmdEntry { name: "try",       func: control::cmd_try,       cat: Language, cmd_id: Some(CmdId::Try      as u16) },
    CmdEntry { name: "catch",     func: control::cmd_catch,     cat: Language, cmd_id: Some(CmdId::Catch    as u16) },
    CmdEntry { name: "proc",      func: proc::cmd_proc,         cat: Language, cmd_id: Some(CmdId::Proc     as u16) },
    CmdEntry { name: "rename",    func: proc::cmd_rename,       cat: Language, cmd_id: Some(CmdId::Rename   as u16) },
    CmdEntry { name: "eval",      func: proc::cmd_eval,         cat: Language, cmd_id: Some(CmdId::Eval     as u16) },
    CmdEntry { name: "apply",     func: proc::cmd_apply,        cat: Language, cmd_id: Some(CmdId::Apply    as u16) },
    CmdEntry { name: "uplevel",   func: proc::cmd_uplevel,      cat: Language, cmd_id: Some(CmdId::Uplevel  as u16) },
    CmdEntry { name: "upvar",     func: proc::cmd_upvar,        cat: Language, cmd_id: Some(CmdId::Upvar    as u16) },
    CmdEntry { name: "global",    func: proc::cmd_global,       cat: Language, cmd_id: Some(CmdId::Global   as u16) },
    CmdEntry { name: "unset",     func: misc::cmd_unset,        cat: Language, cmd_id: Some(CmdId::Unset    as u16) },
    CmdEntry { name: "subst",     func: misc::cmd_subst,        cat: Language, cmd_id: Some(CmdId::Subst    as u16) },
    CmdEntry { name: "info",      func: misc::cmd_info,         cat: Language, cmd_id: Some(CmdId::Info     as u16) },
    CmdEntry { name: "error",     func: control::cmd_error,     cat: Language, cmd_id: Some(CmdId::Error    as u16) },
    CmdEntry { name: "tailcall",  func: control::cmd_tailcall,  cat: Language, cmd_id: Some(CmdId::Tailcall as u16) },
    CmdEntry { name: "append",    func: misc::cmd_append,       cat: Language, cmd_id: Some(CmdId::Append   as u16) },
    // ── Standard library ───────────────────────────────────────────────────
    CmdEntry { name: "string",    func: string_cmds::cmd_string, cat: Standard, cmd_id: Some(CmdId::StringCmd as u16) },
    CmdEntry { name: "list",      func: list::cmd_list,          cat: Standard, cmd_id: Some(CmdId::List      as u16) },
    CmdEntry { name: "llength",   func: list::cmd_llength,       cat: Standard, cmd_id: Some(CmdId::Llength   as u16) },
    CmdEntry { name: "lindex",    func: list::cmd_lindex,        cat: Standard, cmd_id: Some(CmdId::Lindex    as u16) },
    CmdEntry { name: "lappend",   func: list::cmd_lappend,       cat: Standard, cmd_id: Some(CmdId::Lappend   as u16) },
    CmdEntry { name: "lrange",    func: list::cmd_lrange,        cat: Standard, cmd_id: Some(CmdId::Lrange    as u16) },
    CmdEntry { name: "lsearch",   func: list::cmd_lsearch,       cat: Standard, cmd_id: Some(CmdId::Lsearch   as u16) },
    CmdEntry { name: "lsort",     func: list::cmd_lsort,         cat: Standard, cmd_id: Some(CmdId::Lsort     as u16) },
    CmdEntry { name: "linsert",   func: list::cmd_linsert,       cat: Standard, cmd_id: Some(CmdId::Linsert   as u16) },
    CmdEntry { name: "lreplace",  func: list::cmd_lreplace,      cat: Standard, cmd_id: Some(CmdId::Lreplace  as u16) },
    CmdEntry { name: "lassign",   func: list::cmd_lassign,       cat: Standard, cmd_id: Some(CmdId::Lassign   as u16) },
    CmdEntry { name: "lrepeat",   func: list::cmd_lrepeat,       cat: Standard, cmd_id: Some(CmdId::Lrepeat   as u16) },
    CmdEntry { name: "lreverse",  func: list::cmd_lreverse,      cat: Standard, cmd_id: Some(CmdId::Lreverse  as u16) },
    CmdEntry { name: "concat",    func: list::cmd_concat,        cat: Standard, cmd_id: Some(CmdId::Concat    as u16) },
    CmdEntry { name: "split",     func: list::cmd_split,         cat: Standard, cmd_id: Some(CmdId::Split     as u16) },
    CmdEntry { name: "join",      func: list::cmd_join,          cat: Standard, cmd_id: Some(CmdId::Join      as u16) },
    CmdEntry { name: "lmap",      func: list::cmd_lmap,          cat: Standard, cmd_id: Some(CmdId::Lmap      as u16) },
    CmdEntry { name: "lset",      func: list::cmd_lset,          cat: Standard, cmd_id: Some(CmdId::Lset      as u16) },
    CmdEntry { name: "dict",      func: dict::cmd_dict,          cat: Standard, cmd_id: Some(CmdId::Dict      as u16) },
    CmdEntry { name: "array",     func: array::cmd_array,        cat: Standard, cmd_id: Some(CmdId::Array     as u16) },
    CmdEntry { name: "format",    func: io::cmd_format,          cat: Standard, cmd_id: Some(CmdId::Format    as u16) },
    CmdEntry { name: "scan",      func: misc::cmd_scan,          cat: Standard, cmd_id: Some(CmdId::Scan      as u16) },
    CmdEntry { name: "range",     func: loops::cmd_range,        cat: Standard, cmd_id: Some(CmdId::Range     as u16) },
    CmdEntry { name: "time",      func: loops::cmd_time,         cat: Standard, cmd_id: Some(CmdId::Time      as u16) },
    CmdEntry { name: "timerate",  func: loops::cmd_timerate,     cat: Standard, cmd_id: Some(CmdId::Timerate  as u16) },
    CmdEntry { name: "clock",     func: clock::cmd_clock,        cat: Standard, cmd_id: Some(CmdId::Clock    as u16) },
    CmdEntry { name: "package",   func: package::cmd_package,    cat: Standard, cmd_id: Some(CmdId::Package  as u16) },
    CmdEntry { name: "namespace", func: namespace::cmd_namespace, cat: Language,  cmd_id: Some(CmdId::Namespace as u16) },
    CmdEntry { name: "variable",  func: namespace::cmd_variable,  cat: Language,  cmd_id: Some(CmdId::Variable as u16) },
    // ── Extension commands (always available) ──────────────────────────────
    CmdEntry { name: "puts",        func: io::cmd_puts,           cat: Extension, cmd_id: Some(CmdId::Puts        as u16) },
    CmdEntry { name: "disassemble", func: misc::cmd_disassemble,  cat: Extension, cmd_id: Some(CmdId::Disassemble as u16) },
];

/// Extension commands gated behind `feature = "std"`.
#[cfg(feature = "std")]
static CMD_TABLE_STD: &[CmdEntry] = &[
    CmdEntry { name: "source",  func: io::cmd_source,            cat: Extension, cmd_id: Some(CmdId::Source as u16) },
    CmdEntry { name: "file",    func: io::cmd_file,              cat: Extension, cmd_id: Some(CmdId::File   as u16) },
    CmdEntry { name: "glob",    func: io::cmd_glob,              cat: Extension, cmd_id: Some(CmdId::Glob   as u16) },
    CmdEntry { name: "regexp",  func: regexp_cmds::cmd_regexp,   cat: Extension, cmd_id: Some(CmdId::Regexp as u16) },
    CmdEntry { name: "regsub",  func: regexp_cmds::cmd_regsub,   cat: Extension, cmd_id: Some(CmdId::Regsub as u16) },
    CmdEntry { name: "open",   func: chan_io::cmd_open,          cat: Extension, cmd_id: Some(CmdId::Open   as u16) },
    CmdEntry { name: "close",  func: chan_io::cmd_close,         cat: Extension, cmd_id: Some(CmdId::Close  as u16) },
    CmdEntry { name: "read",   func: chan_io::cmd_read,          cat: Extension, cmd_id: Some(CmdId::Read   as u16) },
    CmdEntry { name: "gets",   func: chan_io::cmd_gets,          cat: Extension, cmd_id: Some(CmdId::Gets   as u16) },
    CmdEntry { name: "seek",   func: chan_io::cmd_seek,          cat: Extension, cmd_id: Some(CmdId::Seek   as u16) },
    CmdEntry { name: "tell",   func: chan_io::cmd_tell,          cat: Extension, cmd_id: Some(CmdId::Tell   as u16) },
    CmdEntry { name: "eof",    func: chan_io::cmd_eof,           cat: Extension, cmd_id: Some(CmdId::Eof    as u16) },
    CmdEntry { name: "flush",  func: chan_io::cmd_flush,         cat: Extension, cmd_id: Some(CmdId::Flush  as u16) },
    CmdEntry { name: "fconfigure", func: chan_io::cmd_fconfigure, cat: Extension, cmd_id: Some(CmdId::Fconfigure as u16) },
    CmdEntry { name: "pid",    func: chan_io::cmd_pid,           cat: Extension, cmd_id: Some(CmdId::Pid    as u16) },
    CmdEntry { name: "exec",   func: exec_cmd::cmd_exec,        cat: Extension, cmd_id: Some(CmdId::Exec   as u16) },
];

impl Interp {
    /// Register all built-in commands from the master table.
    pub(super) fn register_builtins(&mut self) {
        for entry in CMD_TABLE {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "std")]
        for entry in CMD_TABLE_STD {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
    }

    // -- Command registration ------------------------------------------------

    /// Register an external command (always categorised as Extension).
    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
        self.command_categories.insert(name.to_string(), CommandCategory::Extension);
    }

    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        self.command_categories.remove(name);
        Ok(())
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    // -- Call dispatch (by numeric CmdId) ------------------------------------

    /// Map a `CmdId` (u16) to the corresponding command function.
    /// Derived from the same master table as `register_builtins()`.
    pub(crate) fn resolve_cmd(&self, cmd_id: u16) -> Option<CommandFunc> {
        for entry in CMD_TABLE {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        #[cfg(feature = "std")]
        for entry in CMD_TABLE_STD {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        None
    }

    /// Return the category for a command, if it exists.
    pub fn command_category(&self, name: &str) -> Option<CommandCategory> {
        self.command_categories.get(name).copied()
    }
}
