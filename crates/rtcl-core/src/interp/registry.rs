//! Command registry — single-source command table, built-in registration, and
//! CmdId dispatch.

use super::Interp;
use crate::command::{CommandFunc, CommandCategory, CommandMeta};
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
    /// Usage signature shown by `info usage`.
    usage: &'static str,
    /// One-line help text shown by `info help`.
    help: &'static str,
}

/// Master command table — single source of truth for built-in commands.
/// Both `register_builtins()` and `resolve_cmd()` are derived from this table.
/// Commands that use native VM opcodes (set, if, while, for, break, continue,
/// return, exit, expr, incr) have `cmd_id: None`.
static CMD_TABLE: &[CmdEntry] = &[
    // ── Language builtins ──────────────────────────────────────────────────
    CmdEntry { name: "set",       func: misc::cmd_set,          cat: Language, cmd_id: None,                             usage: "varName ?value?",                  help: "Read or write a variable" },
    CmdEntry { name: "if",        func: control::cmd_if,        cat: Language, cmd_id: None,                             usage: "expr1 ?then? body1 ?elseif expr2 body2 ...? ?else bodyN?", help: "Conditional branching" },
    CmdEntry { name: "while",     func: loops::cmd_while,       cat: Language, cmd_id: None,                             usage: "test body",                        help: "Loop while condition is true" },
    CmdEntry { name: "for",       func: loops::cmd_for,         cat: Language, cmd_id: None,                             usage: "init test next body",              help: "C-style for loop" },
    CmdEntry { name: "break",     func: control::cmd_break,     cat: Language, cmd_id: None,                             usage: "?level?",                          help: "Break out of a loop" },
    CmdEntry { name: "continue",  func: control::cmd_continue,  cat: Language, cmd_id: None,                             usage: "?level?",                          help: "Skip to next loop iteration" },
    CmdEntry { name: "return",    func: control::cmd_return,    cat: Language, cmd_id: None,                             usage: "?-code code? ?-level n? ?value?",  help: "Return from a procedure" },
    CmdEntry { name: "exit",      func: control::cmd_exit,      cat: Language, cmd_id: None,                             usage: "?code?",                           help: "Exit the interpreter" },
    CmdEntry { name: "expr",      func: misc::cmd_expr,         cat: Language, cmd_id: None,                             usage: "arg ?arg ...?",                    help: "Evaluate an expression" },
    CmdEntry { name: "incr",      func: misc::cmd_incr,         cat: Language, cmd_id: None,                             usage: "varName ?increment?",              help: "Increment a variable" },
    CmdEntry { name: "foreach",   func: loops::cmd_foreach,     cat: Language, cmd_id: Some(CmdId::Foreach  as u16),     usage: "varList list ?varList list ...? body", help: "Iterate over list elements" },
    CmdEntry { name: "switch",    func: control::cmd_switch,    cat: Language, cmd_id: Some(CmdId::Switch   as u16),     usage: "?options? string {pattern body ...}", help: "Pattern matching" },
    CmdEntry { name: "try",       func: control::cmd_try,       cat: Language, cmd_id: Some(CmdId::Try      as u16),     usage: "body ?on code varList body ...? ?finally body?", help: "Exception handling" },
    CmdEntry { name: "catch",     func: control::cmd_catch,     cat: Language, cmd_id: Some(CmdId::Catch    as u16),     usage: "script ?resultVar? ?optionsVar?",  help: "Catch exceptions" },
    CmdEntry { name: "proc",      func: proc::cmd_proc,         cat: Language, cmd_id: Some(CmdId::Proc     as u16),     usage: "name args body",                   help: "Define a procedure" },
    CmdEntry { name: "rename",    func: proc::cmd_rename,       cat: Language, cmd_id: Some(CmdId::Rename   as u16),     usage: "oldName newName",                  help: "Rename or delete a command" },
    CmdEntry { name: "eval",      func: proc::cmd_eval,         cat: Language, cmd_id: Some(CmdId::Eval     as u16),     usage: "arg ?arg ...?",                    help: "Evaluate a Tcl script" },
    CmdEntry { name: "apply",     func: proc::cmd_apply,        cat: Language, cmd_id: Some(CmdId::Apply    as u16),     usage: "lambdaExpr ?arg ...?",             help: "Apply an anonymous function" },
    CmdEntry { name: "uplevel",   func: proc::cmd_uplevel,      cat: Language, cmd_id: Some(CmdId::Uplevel  as u16),     usage: "?level? arg ?arg ...?",            help: "Execute in caller's scope" },
    CmdEntry { name: "upvar",     func: proc::cmd_upvar,        cat: Language, cmd_id: Some(CmdId::Upvar    as u16),     usage: "?level? otherVar myVar ?...?",     help: "Link to variable in caller's scope" },
    CmdEntry { name: "global",    func: proc::cmd_global,       cat: Language, cmd_id: Some(CmdId::Global   as u16),     usage: "varName ?varName ...?",            help: "Access global variables" },
    CmdEntry { name: "unset",     func: misc::cmd_unset,        cat: Language, cmd_id: Some(CmdId::Unset    as u16),     usage: "?-nocomplain? ?--? varName ?varName ...?", help: "Delete variables" },
    CmdEntry { name: "subst",     func: misc::cmd_subst,        cat: Language, cmd_id: Some(CmdId::Subst    as u16),     usage: "?-nobackslashes? ?-nocommands? ?-novariables? string", help: "Perform substitutions" },
    CmdEntry { name: "info",      func: misc::cmd_info,         cat: Language, cmd_id: Some(CmdId::Info     as u16),     usage: "subcommand ?arg ...?",             help: "Interpreter introspection" },
    CmdEntry { name: "error",     func: control::cmd_error,     cat: Language, cmd_id: Some(CmdId::Error    as u16),     usage: "message ?info? ?code?",            help: "Raise an error" },
    CmdEntry { name: "tailcall",  func: control::cmd_tailcall,  cat: Language, cmd_id: Some(CmdId::Tailcall as u16),     usage: "command ?arg ...?",                help: "Tail call optimisation" },
    CmdEntry { name: "append",    func: misc::cmd_append,       cat: Language, cmd_id: Some(CmdId::Append   as u16),     usage: "varName ?value ...?",              help: "Append to a variable" },
    // ── Standard library ───────────────────────────────────────────────────
    CmdEntry { name: "string",    func: string_cmds::cmd_string, cat: Standard, cmd_id: Some(CmdId::StringCmd as u16),   usage: "subcommand ?arg ...?",             help: "String operations" },
    CmdEntry { name: "list",      func: list::cmd_list,          cat: Standard, cmd_id: Some(CmdId::List      as u16),   usage: "?arg ...?",                        help: "Create a list" },
    CmdEntry { name: "llength",   func: list::cmd_llength,       cat: Standard, cmd_id: Some(CmdId::Llength   as u16),   usage: "list",                             help: "Length of a list" },
    CmdEntry { name: "lindex",    func: list::cmd_lindex,        cat: Standard, cmd_id: Some(CmdId::Lindex    as u16),   usage: "list ?index ...?",                 help: "Get list element by index" },
    CmdEntry { name: "lappend",   func: list::cmd_lappend,       cat: Standard, cmd_id: Some(CmdId::Lappend   as u16),   usage: "varName ?value ...?",              help: "Append elements to a list variable" },
    CmdEntry { name: "lrange",    func: list::cmd_lrange,        cat: Standard, cmd_id: Some(CmdId::Lrange    as u16),   usage: "list first last",                  help: "Extract a range from a list" },
    CmdEntry { name: "lsearch",   func: list_sort::cmd_lsearch,  cat: Standard, cmd_id: Some(CmdId::Lsearch   as u16),   usage: "?options? list pattern",           help: "Search a list" },
    CmdEntry { name: "lsort",     func: list_sort::cmd_lsort,    cat: Standard, cmd_id: Some(CmdId::Lsort     as u16),   usage: "?options? list",                   help: "Sort a list" },
    CmdEntry { name: "linsert",   func: list::cmd_linsert,       cat: Standard, cmd_id: Some(CmdId::Linsert   as u16),   usage: "list index ?element ...?",         help: "Insert elements into a list" },
    CmdEntry { name: "lreplace",  func: list::cmd_lreplace,      cat: Standard, cmd_id: Some(CmdId::Lreplace  as u16),   usage: "list first last ?element ...?",    help: "Replace elements in a list" },
    CmdEntry { name: "lassign",   func: list::cmd_lassign,       cat: Standard, cmd_id: Some(CmdId::Lassign   as u16),   usage: "list ?varName ...?",               help: "Assign list elements to variables" },
    CmdEntry { name: "lrepeat",   func: list::cmd_lrepeat,       cat: Standard, cmd_id: Some(CmdId::Lrepeat   as u16),   usage: "count ?element ...?",              help: "Build a list by repetition" },
    CmdEntry { name: "lreverse",  func: list::cmd_lreverse,      cat: Standard, cmd_id: Some(CmdId::Lreverse  as u16),   usage: "list",                             help: "Reverse a list" },
    CmdEntry { name: "concat",    func: list::cmd_concat,        cat: Standard, cmd_id: Some(CmdId::Concat    as u16),   usage: "?arg ...?",                        help: "Concatenate arguments" },
    CmdEntry { name: "split",     func: list::cmd_split,         cat: Standard, cmd_id: Some(CmdId::Split     as u16),   usage: "string ?splitChars?",              help: "Split a string into a list" },
    CmdEntry { name: "join",      func: list::cmd_join,          cat: Standard, cmd_id: Some(CmdId::Join      as u16),   usage: "list ?joinString?",                help: "Join list elements into a string" },
    CmdEntry { name: "lmap",      func: list::cmd_lmap,          cat: Standard, cmd_id: Some(CmdId::Lmap      as u16),   usage: "varList list ?varList list ...? body", help: "Map over list elements" },
    CmdEntry { name: "lset",      func: list::cmd_lset,          cat: Standard, cmd_id: Some(CmdId::Lset      as u16),   usage: "varName ?index ...? value",        help: "Set a list element" },
    CmdEntry { name: "dict",      func: dict::cmd_dict,          cat: Standard, cmd_id: Some(CmdId::Dict      as u16),   usage: "subcommand ?arg ...?",             help: "Dictionary operations" },
    CmdEntry { name: "array",     func: array::cmd_array,        cat: Standard, cmd_id: Some(CmdId::Array     as u16),   usage: "subcommand arrayName ?arg ...?",   help: "Array operations" },
    CmdEntry { name: "format",    func: io::cmd_format,          cat: Standard, cmd_id: Some(CmdId::Format    as u16),   usage: "formatString ?arg ...?",           help: "Format a string (printf-style)" },
    CmdEntry { name: "scan",      func: misc::cmd_scan,          cat: Standard, cmd_id: Some(CmdId::Scan      as u16),   usage: "string format ?varName ...?",      help: "Parse a string (scanf-style)" },
    CmdEntry { name: "range",     func: loops::cmd_range,        cat: Standard, cmd_id: Some(CmdId::Range     as u16),   usage: "?start? end ?step?",               help: "Generate an integer range list" },
    CmdEntry { name: "time",      func: loops::cmd_time,         cat: Standard, cmd_id: Some(CmdId::Time      as u16),   usage: "script ?count?",                   help: "Measure script execution time" },
    CmdEntry { name: "timerate",  func: loops::cmd_timerate,     cat: Standard, cmd_id: Some(CmdId::Timerate  as u16),   usage: "script ?milliseconds?",            help: "Measure script throughput" },
    CmdEntry { name: "namespace", func: namespace::cmd_namespace, cat: Language,  cmd_id: Some(CmdId::Namespace as u16),  usage: "subcommand ?arg ...?",             help: "Namespace operations" },
    CmdEntry { name: "variable",  func: namespace::cmd_variable,  cat: Language,  cmd_id: Some(CmdId::Variable as u16),  usage: "?name value ...? name ?value?",    help: "Declare namespace variable" },
    // ── Extension commands (always available) ──────────────────────────────
    CmdEntry { name: "puts",        func: io::cmd_puts,           cat: Extension, cmd_id: Some(CmdId::Puts        as u16), usage: "?-nonewline? ?channelId? string", help: "Output a string" },
    CmdEntry { name: "disassemble", func: misc::cmd_disassemble,  cat: Extension, cmd_id: Some(CmdId::Disassemble as u16), usage: "procName",                       help: "Show bytecode for a procedure" },
    // ── Introspection commands ─────────────────────────────────────────────
    CmdEntry { name: "exists",     func: introspect::cmd_exists,     cat: Language,  cmd_id: None, usage: "varName",                     help: "Check if a variable exists" },
    CmdEntry { name: "alias",      func: introspect::cmd_alias,      cat: Language,  cmd_id: None, usage: "name target ?arg ...?",       help: "Create a command alias" },
    CmdEntry { name: "local",      func: introspect::cmd_local,      cat: Language,  cmd_id: None, usage: "command ?arg ...?",           help: "Mark command as scope-local" },
    CmdEntry { name: "upcall",     func: introspect::cmd_upcall,     cat: Language,  cmd_id: None, usage: "command ?arg ...?",           help: "Call the original (overridden) command" },
    CmdEntry { name: "unknown",    func: introspect::cmd_unknown,    cat: Language,  cmd_id: None, usage: "cmdName ?arg ...?",           help: "Handler for unknown commands" },
    CmdEntry { name: "defer",      func: introspect::cmd_defer,      cat: Language,  cmd_id: None, usage: "script",                     help: "Run script when scope exits" },
    CmdEntry { name: "ref",        func: introspect::cmd_ref,        cat: Language,  cmd_id: None, usage: "value ?tag?",                help: "Create a reference" },
    CmdEntry { name: "getref",     func: introspect::cmd_getref,     cat: Language,  cmd_id: None, usage: "reference",                  help: "Get value of a reference" },
    CmdEntry { name: "setref",     func: introspect::cmd_setref,     cat: Language,  cmd_id: None, usage: "reference value",            help: "Set value of a reference" },
    CmdEntry { name: "collect",    func: introspect::cmd_collect,    cat: Language,  cmd_id: None, usage: "",                           help: "Collect unreferenced references" },
    CmdEntry { name: "finalize",   func: introspect::cmd_finalize,   cat: Language,  cmd_id: None, usage: "reference ?script?",         help: "Set or get reference finalizer" },
    CmdEntry { name: "stacktrace", func: introspect::cmd_stacktrace, cat: Language,  cmd_id: None, usage: "",                           help: "Return the current call stack" },
    CmdEntry { name: "pack",       func: introspect::cmd_pack,       cat: Standard,  cmd_id: None, usage: "value -intle|-intbe|-floatle|-floatbe|-str width", help: "Pack a value into binary" },
    CmdEntry { name: "unpack",     func: introspect::cmd_unpack,     cat: Standard,  cmd_id: None, usage: "binValue -intle|-intbe|-floatle|-floatbe|-str offset width", help: "Unpack binary data" },
    // ── Arithmetic operator commands (jimtcl core) ─────────────────────────
    CmdEntry { name: "+",         func: misc::cmd_add,              cat: Standard,  cmd_id: None, usage: "number ?number ...?",         help: "Add numbers" },
    CmdEntry { name: "-",         func: misc::cmd_sub,              cat: Standard,  cmd_id: None, usage: "number ?number ...?",         help: "Subtract numbers" },
    CmdEntry { name: "*",         func: misc::cmd_mul,              cat: Standard,  cmd_id: None, usage: "number ?number ...?",         help: "Multiply numbers" },
    CmdEntry { name: "/",         func: misc::cmd_div,              cat: Standard,  cmd_id: None, usage: "number ?number ...?",         help: "Divide numbers" },
    // ── Additional jimtcl core commands ────────────────────────────────────
    CmdEntry { name: "loop",      func: loops::cmd_loop,            cat: Language,  cmd_id: None, usage: "var first limit ?incr? body", help: "Numeric loop (jimtcl extension)" },
    CmdEntry { name: "lsubst",    func: list::cmd_lsubst,           cat: Standard,  cmd_id: None, usage: "list ?-nocase? ?-all? ?--? pattern replacement", help: "Replace elements in a list" },
    CmdEntry { name: "rand",      func: misc::cmd_rand,             cat: Standard,  cmd_id: None, usage: "?min? ?max?",                help: "Generate a random integer" },
    CmdEntry { name: "debug",     func: misc::cmd_debug,            cat: Extension, cmd_id: None, usage: "subcommand ?arg?",           help: "Debugging commands" },
    CmdEntry { name: "xtrace",    func: misc::cmd_xtrace,           cat: Extension, cmd_id: None, usage: "?callback?",                 help: "Set or clear execution trace" },
    CmdEntry { name: "taint",     func: introspect::cmd_taint,      cat: Extension, cmd_id: None, usage: "varName",                    help: "Mark a variable as tainted" },
    CmdEntry { name: "untaint",   func: introspect::cmd_untaint,    cat: Extension, cmd_id: None, usage: "varName",                    help: "Remove taint from a variable" },
    // ── JSON extension ─────────────────────────────────────────────────────
    CmdEntry { name: "json",        func: json::cmd_json,            cat: Extension, cmd_id: None, usage: "subcommand ?arg ...?",       help: "JSON encode/decode" },
    CmdEntry { name: "json::decode", func: json::cmd_json_decode,    cat: Extension, cmd_id: None, usage: "?-index? ?-null string? ?-schema? json-string", help: "Decode JSON to Tcl value" },
    CmdEntry { name: "json::encode", func: json::cmd_json_encode,    cat: Extension, cmd_id: None, usage: "value ?schema?",             help: "Encode Tcl value to JSON" },
];

/// Commands gated behind `feature = "clock"`.
#[cfg(feature = "clock")]
static CMD_TABLE_CLOCK: &[CmdEntry] = &[
    CmdEntry { name: "clock",     func: clock::cmd_clock,        cat: Standard, cmd_id: Some(CmdId::Clock    as u16), usage: "subcommand ?arg ...?", help: "Time and date operations" },
];

/// Package system — gated behind `feature = "package"` (implies `file`).
#[cfg(feature = "package")]
static CMD_TABLE_PKG: &[CmdEntry] = &[
    CmdEntry { name: "package",   func: package::cmd_package,    cat: Standard, cmd_id: Some(CmdId::Package  as u16), usage: "subcommand ?arg ...?", help: "Package management" },
];

/// File system commands gated behind `feature = "file"`.
#[cfg(feature = "file")]
static CMD_TABLE_FILE: &[CmdEntry] = &[
    CmdEntry { name: "source",  func: io::cmd_source,   cat: Extension, cmd_id: Some(CmdId::Source as u16), usage: "fileName",       help: "Evaluate a file as a Tcl script" },
    CmdEntry { name: "file",    func: io::cmd_file,     cat: Extension, cmd_id: Some(CmdId::File   as u16), usage: "subcommand ?arg ...?", help: "File system operations" },
    CmdEntry { name: "glob",    func: io::cmd_glob,     cat: Extension, cmd_id: Some(CmdId::Glob   as u16), usage: "?options? pattern ?pattern ...?", help: "Return filenames matching patterns" },
    CmdEntry { name: "cd",      func: os::cmd_cd,       cat: Extension, cmd_id: None,                       usage: "?dirName?",      help: "Change working directory" },
    CmdEntry { name: "pwd",     func: os::cmd_pwd,      cat: Extension, cmd_id: None,                       usage: "",               help: "Return current working directory" },
    CmdEntry { name: "readdir", func: os::cmd_readdir,  cat: Extension, cmd_id: None,                       usage: "dirName",        help: "List directory contents" },
];

/// Channel I/O commands gated behind `feature = "io"`.
#[cfg(feature = "io")]
static CMD_TABLE_IO: &[CmdEntry] = &[
    CmdEntry { name: "open",        func: chan_io::cmd_open,        cat: Extension, cmd_id: Some(CmdId::Open   as u16),        usage: "fileName ?access? ?permissions?", help: "Open a file or pipe" },
    CmdEntry { name: "close",       func: chan_io::cmd_close,       cat: Extension, cmd_id: Some(CmdId::Close  as u16),        usage: "channelId",              help: "Close a channel" },
    CmdEntry { name: "read",        func: chan_io::cmd_read,        cat: Extension, cmd_id: Some(CmdId::Read   as u16),        usage: "?-nonewline? channelId ?numChars?", help: "Read from a channel" },
    CmdEntry { name: "gets",        func: chan_io::cmd_gets,        cat: Extension, cmd_id: Some(CmdId::Gets   as u16),        usage: "channelId ?varName?",    help: "Read a line from a channel" },
    CmdEntry { name: "seek",        func: chan_io::cmd_seek,        cat: Extension, cmd_id: Some(CmdId::Seek   as u16),        usage: "channelId offset ?origin?", help: "Set channel position" },
    CmdEntry { name: "tell",        func: chan_io::cmd_tell,        cat: Extension, cmd_id: Some(CmdId::Tell   as u16),        usage: "channelId",              help: "Get channel position" },
    CmdEntry { name: "eof",         func: chan_io::cmd_eof,         cat: Extension, cmd_id: Some(CmdId::Eof    as u16),        usage: "channelId",              help: "Check for end of file" },
    CmdEntry { name: "flush",       func: chan_io::cmd_flush,       cat: Extension, cmd_id: Some(CmdId::Flush  as u16),        usage: "channelId",              help: "Flush channel output" },
    CmdEntry { name: "fconfigure",  func: chan_io::cmd_fconfigure,  cat: Extension, cmd_id: Some(CmdId::Fconfigure as u16),    usage: "channelId ?name? ?value ...?", help: "Configure channel options" },
    CmdEntry { name: "pid",         func: chan_io::cmd_pid,         cat: Extension, cmd_id: Some(CmdId::Pid    as u16),        usage: "?channelId?",            help: "Get process ID" },
];

/// Process execution commands gated behind `feature = "exec"`.
#[cfg(feature = "exec")]
static CMD_TABLE_EXEC: &[CmdEntry] = &[
    CmdEntry { name: "exec",  func: exec_cmd::cmd_exec, cat: Extension, cmd_id: Some(CmdId::Exec as u16), usage: "?-ignorestderr? ?-keepnewline? ?--? arg ?arg ...?", help: "Execute a system command with I/O redirections" },
    CmdEntry { name: "wait",  func: os::cmd_wait,       cat: Extension, cmd_id: None,                     usage: "?-nohang? ?pid?",  help: "Wait for a process" },
];

/// Regular expression commands gated behind `feature = "regexp"`.
#[cfg(feature = "regexp")]
static CMD_TABLE_REGEXP: &[CmdEntry] = &[
    CmdEntry { name: "regexp", func: regexp_cmds::cmd_regexp, cat: Extension, cmd_id: Some(CmdId::Regexp as u16), usage: "?-nocase? ?-all? ?-inline? ?-indices? ?-expanded? ?-line? ?-start offset? ?--? exp string ?matchVar? ?subMatchVar ...?", help: "Regular expression matching" },
    CmdEntry { name: "regsub", func: regexp_cmds::cmd_regsub, cat: Extension, cmd_id: Some(CmdId::Regsub as u16), usage: "?-nocase? ?-all? ?-expanded? ?-line? ?-start offset? ?-command? ?--? exp string subSpec ?varName?", help: "Regular expression substitution" },
];

/// Signal/process control commands gated behind `feature = "signal"`.
#[cfg(feature = "signal")]
static CMD_TABLE_SIGNAL: &[CmdEntry] = &[
    CmdEntry { name: "sleep", func: os::cmd_sleep, cat: Extension, cmd_id: None, usage: "seconds",        help: "Sleep for N seconds" },
    CmdEntry { name: "kill",  func: os::cmd_kill,  cat: Extension, cmd_id: None, usage: "?signal? pid",   help: "Send a signal to a process" },
];

/// Environment variable command gated behind `feature = "env"`.
#[cfg(feature = "env")]
static CMD_TABLE_ENV: &[CmdEntry] = &[
    CmdEntry { name: "env", func: misc::cmd_env, cat: Standard, cmd_id: None, usage: "varName ?value?", help: "Read or write environment variables" },
];

/// Event loop commands gated behind `feature = "std"`.
#[cfg(feature = "std")]
static CMD_TABLE_EVENT: &[CmdEntry] = &[
    CmdEntry { name: "after",   func: event::cmd_after,   cat: Extension, cmd_id: Some(CmdId::After  as u16), usage: "option ?arg ...?",    help: "Schedule or sleep" },
    CmdEntry { name: "vwait",   func: event::cmd_vwait,   cat: Extension, cmd_id: Some(CmdId::Vwait  as u16), usage: "varName",             help: "Wait for variable change" },
    CmdEntry { name: "update",  func: event::cmd_update,  cat: Extension, cmd_id: Some(CmdId::Update as u16), usage: "?idletasks?",         help: "Process pending events" },
    CmdEntry { name: "interp",  func: interp_cmd::cmd_interp, cat: Extension, cmd_id: Some(CmdId::InterpCmd as u16), usage: "", help: "Create a child interpreter" },
];

impl Interp {
    /// Insert a `CmdEntry` into the interpreter tables.
    fn register_entry(&mut self, entry: &CmdEntry) {
        self.commands.insert(entry.name.to_string(), entry.func);
        self.command_categories.insert(entry.name.to_string(), entry.cat);
        self.command_meta.insert(entry.name.to_string(), CommandMeta {
            usage: entry.usage,
            help: entry.help,
        });
    }

    /// Register all built-in commands from the master table.
    pub(super) fn register_builtins(&mut self) {
        for entry in CMD_TABLE {
            self.register_entry(entry);
        }
        #[cfg(feature = "package")]
        for entry in CMD_TABLE_PKG {
            self.register_entry(entry);
        }
        #[cfg(feature = "clock")]
        for entry in CMD_TABLE_CLOCK {
            self.register_entry(entry);
        }
        #[cfg(feature = "file")]
        for entry in CMD_TABLE_FILE {
            self.register_entry(entry);
        }
        #[cfg(feature = "io")]
        for entry in CMD_TABLE_IO {
            self.register_entry(entry);
        }
        #[cfg(feature = "exec")]
        for entry in CMD_TABLE_EXEC {
            self.register_entry(entry);
        }
        #[cfg(feature = "regexp")]
        for entry in CMD_TABLE_REGEXP {
            self.register_entry(entry);
        }
        #[cfg(feature = "signal")]
        for entry in CMD_TABLE_SIGNAL {
            self.register_entry(entry);
        }
        #[cfg(feature = "env")]
        for entry in CMD_TABLE_ENV {
            self.register_entry(entry);
        }
        #[cfg(feature = "std")]
        for entry in CMD_TABLE_EVENT {
            self.register_entry(entry);
        }
    }

    // -- Command registration ------------------------------------------------

    /// Register an external command (always categorised as Extension).
    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
        self.command_categories.insert(name.to_string(), CommandCategory::Extension);
    }

    /// Register an external command with metadata (usage + help).
    pub fn register_command_with_meta(
        &mut self,
        name: &str,
        func: CommandFunc,
        meta: CommandMeta,
    ) {
        self.commands.insert(name.to_string(), func);
        self.command_categories.insert(name.to_string(), CommandCategory::Extension);
        self.command_meta.insert(name.to_string(), meta);
    }

    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        self.command_categories.remove(name);
        self.command_meta.remove(name);
        Ok(())
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Return the usage signature for a command (e.g. `"lsort ?options? list"`).
    ///
    /// For procs, the usage is auto-generated from the parameter list.
    /// Returns `None` if the command does not exist.
    pub fn command_usage(&self, name: &str) -> Option<String> {
        // Check proc definitions first — auto-generate usage from arglist
        if let Some(proc_def) = self.procs.get(name) {
            let mut parts = Vec::new();
            for (param, default) in &proc_def.params {
                if param == "args" {
                    parts.push("?arg ...?".to_string());
                } else if default.is_some() {
                    parts.push(format!("?{}?", param));
                } else {
                    parts.push(param.clone());
                }
            }
            return Some(parts.join(" "));
        }
        // Then check native command metadata
        if let Some(meta) = self.command_meta.get(name) {
            return Some(meta.usage.to_string());
        }
        // Command exists but has no metadata
        if self.commands.contains_key(name) {
            return Some(String::new());
        }
        None
    }

    /// Return the help text for a command.
    ///
    /// Returns `None` if the command does not exist.
    pub fn command_help(&self, name: &str) -> Option<String> {
        if let Some(meta) = self.command_meta.get(name) {
            return Some(meta.help.to_string());
        }
        if self.commands.contains_key(name) || self.procs.contains_key(name) {
            return Some(String::new());
        }
        None
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
        #[cfg(feature = "package")]
        for entry in CMD_TABLE_PKG {
            if entry.cmd_id == Some(cmd_id) {
                return Some(entry.func);
            }
        }
        #[cfg(feature = "std")]
        for entry in CMD_TABLE_EVENT {
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
