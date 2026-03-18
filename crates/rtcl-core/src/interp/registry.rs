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
    CmdEntry { name: "namespace", func: namespace::cmd_namespace, cat: Language,  cmd_id: Some(CmdId::Namespace as u16) },
    CmdEntry { name: "variable",  func: namespace::cmd_variable,  cat: Language,  cmd_id: Some(CmdId::Variable as u16) },
    // ── Extension commands (always available) ──────────────────────────────
    CmdEntry { name: "puts",        func: io::cmd_puts,           cat: Extension, cmd_id: Some(CmdId::Puts        as u16) },
    CmdEntry { name: "disassemble", func: misc::cmd_disassemble,  cat: Extension, cmd_id: Some(CmdId::Disassemble as u16) },
    // ── Introspection commands ─────────────────────────────────────────────
    CmdEntry { name: "exists",     func: introspect::cmd_exists,     cat: Language,  cmd_id: None },
    CmdEntry { name: "alias",      func: introspect::cmd_alias,      cat: Language,  cmd_id: None },
    CmdEntry { name: "local",      func: introspect::cmd_local,      cat: Language,  cmd_id: None },
    CmdEntry { name: "upcall",     func: introspect::cmd_upcall,     cat: Language,  cmd_id: None },
    CmdEntry { name: "unknown",    func: introspect::cmd_unknown,    cat: Language,  cmd_id: None },
    CmdEntry { name: "defer",      func: introspect::cmd_defer,      cat: Language,  cmd_id: None },
    CmdEntry { name: "ref",        func: introspect::cmd_ref,        cat: Language,  cmd_id: None },
    CmdEntry { name: "getref",     func: introspect::cmd_getref,     cat: Language,  cmd_id: None },
    CmdEntry { name: "setref",     func: introspect::cmd_setref,     cat: Language,  cmd_id: None },
    CmdEntry { name: "collect",    func: introspect::cmd_collect,    cat: Language,  cmd_id: None },
    CmdEntry { name: "finalize",   func: introspect::cmd_finalize,   cat: Language,  cmd_id: None },
    CmdEntry { name: "stacktrace", func: introspect::cmd_stacktrace, cat: Language,  cmd_id: None },
    CmdEntry { name: "pack",       func: introspect::cmd_pack,       cat: Standard,  cmd_id: None },
    CmdEntry { name: "unpack",     func: introspect::cmd_unpack,     cat: Standard,  cmd_id: None },
    // ── Arithmetic operator commands (jimtcl core) ─────────────────────────
    CmdEntry { name: "+",         func: misc::cmd_add,              cat: Standard,  cmd_id: None },
    CmdEntry { name: "-",         func: misc::cmd_sub,              cat: Standard,  cmd_id: None },
    CmdEntry { name: "*",         func: misc::cmd_mul,              cat: Standard,  cmd_id: None },
    CmdEntry { name: "/",         func: misc::cmd_div,              cat: Standard,  cmd_id: None },
    // ── Additional jimtcl core commands ────────────────────────────────────
    CmdEntry { name: "loop",      func: loops::cmd_loop,            cat: Language,  cmd_id: None },
    CmdEntry { name: "lsubst",    func: list::cmd_lsubst,           cat: Standard,  cmd_id: None },
    CmdEntry { name: "rand",      func: misc::cmd_rand,             cat: Standard,  cmd_id: None },
    CmdEntry { name: "debug",     func: misc::cmd_debug,            cat: Extension, cmd_id: None },
    CmdEntry { name: "xtrace",    func: misc::cmd_xtrace,           cat: Extension, cmd_id: None },
    CmdEntry { name: "taint",     func: introspect::cmd_taint,      cat: Extension, cmd_id: None },
    CmdEntry { name: "untaint",   func: introspect::cmd_untaint,    cat: Extension, cmd_id: None },
];

/// Commands gated behind `feature = "clock"`.
#[cfg(feature = "clock")]
static CMD_TABLE_CLOCK: &[CmdEntry] = &[
    CmdEntry { name: "clock",     func: clock::cmd_clock,        cat: Standard, cmd_id: Some(CmdId::Clock    as u16) },
];

/// Package system (always available — pure data for provide/names/forget;
/// `require` auto-load needs `std` at runtime but fails gracefully without it).
static CMD_TABLE_PKG: &[CmdEntry] = &[
    CmdEntry { name: "package",   func: package::cmd_package,    cat: Standard, cmd_id: Some(CmdId::Package  as u16) },
];

/// File system commands gated behind `feature = "file"`.
#[cfg(feature = "file")]
static CMD_TABLE_FILE: &[CmdEntry] = &[
    CmdEntry { name: "source",  func: io::cmd_source,   cat: Extension, cmd_id: Some(CmdId::Source as u16) },
    CmdEntry { name: "file",    func: io::cmd_file,     cat: Extension, cmd_id: Some(CmdId::File   as u16) },
    CmdEntry { name: "glob",    func: io::cmd_glob,     cat: Extension, cmd_id: Some(CmdId::Glob   as u16) },
    CmdEntry { name: "cd",      func: os::cmd_cd,       cat: Extension, cmd_id: None },
    CmdEntry { name: "pwd",     func: os::cmd_pwd,      cat: Extension, cmd_id: None },
    CmdEntry { name: "readdir", func: os::cmd_readdir,  cat: Extension, cmd_id: None },
];

/// Channel I/O commands gated behind `feature = "io"`.
#[cfg(feature = "io")]
static CMD_TABLE_IO: &[CmdEntry] = &[
    CmdEntry { name: "open",        func: chan_io::cmd_open,        cat: Extension, cmd_id: Some(CmdId::Open   as u16) },
    CmdEntry { name: "close",       func: chan_io::cmd_close,       cat: Extension, cmd_id: Some(CmdId::Close  as u16) },
    CmdEntry { name: "read",        func: chan_io::cmd_read,        cat: Extension, cmd_id: Some(CmdId::Read   as u16) },
    CmdEntry { name: "gets",        func: chan_io::cmd_gets,        cat: Extension, cmd_id: Some(CmdId::Gets   as u16) },
    CmdEntry { name: "seek",        func: chan_io::cmd_seek,        cat: Extension, cmd_id: Some(CmdId::Seek   as u16) },
    CmdEntry { name: "tell",        func: chan_io::cmd_tell,        cat: Extension, cmd_id: Some(CmdId::Tell   as u16) },
    CmdEntry { name: "eof",         func: chan_io::cmd_eof,         cat: Extension, cmd_id: Some(CmdId::Eof    as u16) },
    CmdEntry { name: "flush",       func: chan_io::cmd_flush,       cat: Extension, cmd_id: Some(CmdId::Flush  as u16) },
    CmdEntry { name: "fconfigure",  func: chan_io::cmd_fconfigure,  cat: Extension, cmd_id: Some(CmdId::Fconfigure as u16) },
    CmdEntry { name: "pid",         func: chan_io::cmd_pid,         cat: Extension, cmd_id: Some(CmdId::Pid    as u16) },
];

/// Process execution commands gated behind `feature = "exec"`.
#[cfg(feature = "exec")]
static CMD_TABLE_EXEC: &[CmdEntry] = &[
    CmdEntry { name: "exec",  func: exec_cmd::cmd_exec, cat: Extension, cmd_id: Some(CmdId::Exec as u16) },
    CmdEntry { name: "wait",  func: os::cmd_wait,       cat: Extension, cmd_id: None },
];

/// Regular expression commands gated behind `feature = "regexp"`.
#[cfg(feature = "regexp")]
static CMD_TABLE_REGEXP: &[CmdEntry] = &[
    CmdEntry { name: "regexp", func: regexp_cmds::cmd_regexp, cat: Extension, cmd_id: Some(CmdId::Regexp as u16) },
    CmdEntry { name: "regsub", func: regexp_cmds::cmd_regsub, cat: Extension, cmd_id: Some(CmdId::Regsub as u16) },
];

/// Signal/process control commands gated behind `feature = "signal"`.
#[cfg(feature = "signal")]
static CMD_TABLE_SIGNAL: &[CmdEntry] = &[
    CmdEntry { name: "sleep", func: os::cmd_sleep, cat: Extension, cmd_id: None },
    CmdEntry { name: "kill",  func: os::cmd_kill,  cat: Extension, cmd_id: None },
];

/// Environment variable command gated behind `feature = "env"`.
#[cfg(feature = "env")]
static CMD_TABLE_ENV: &[CmdEntry] = &[
    CmdEntry { name: "env", func: misc::cmd_env, cat: Standard, cmd_id: None },
];

impl Interp {
    /// Register all built-in commands from the master table.
    pub(super) fn register_builtins(&mut self) {
        for entry in CMD_TABLE {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        for entry in CMD_TABLE_PKG {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "clock")]
        for entry in CMD_TABLE_CLOCK {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "file")]
        for entry in CMD_TABLE_FILE {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "io")]
        for entry in CMD_TABLE_IO {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "exec")]
        for entry in CMD_TABLE_EXEC {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "regexp")]
        for entry in CMD_TABLE_REGEXP {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "signal")]
        for entry in CMD_TABLE_SIGNAL {
            self.commands.insert(entry.name.to_string(), entry.func);
            self.command_categories.insert(entry.name.to_string(), entry.cat);
        }
        #[cfg(feature = "env")]
        for entry in CMD_TABLE_ENV {
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
        #[cfg(feature = "file")]
        for entry in CMD_TABLE_FILE {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        #[cfg(feature = "io")]
        for entry in CMD_TABLE_IO {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        #[cfg(feature = "exec")]
        for entry in CMD_TABLE_EXEC {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        #[cfg(feature = "regexp")]
        for entry in CMD_TABLE_REGEXP {
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
