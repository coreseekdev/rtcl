//! Tcl interpreter — executes parsed commands.
//!
//! The interpreter is split into:
//! - This file: [`Interp`] struct definition and constructor
//! - [`registry`]: built-in command registration and CmdId dispatch
//! - [`vars`]: variable access methods
//! - [`eval`]: script evaluation and word expansion
//! - [`call`]: procedure calls and tail-call optimisation
//! - [`vm_bridge`]: [`VmContext`](rtcl_vm::VmContext) implementation
//! - [`util`]: shared helpers (`split_array_ref`, `glob_match`)
//! - [`commands`] submodules: individual command implementations

pub mod commands;
pub mod unicode;
mod registry;
mod vars;
mod eval;
mod call;
mod vm_bridge;
mod util;

// Re-export utilities so command modules can reach them via `super::super::glob_match`
pub(crate) use util::{split_array_ref, glob_match};

use crate::command::{CommandFunc, CommandCategory, CommandMeta};
use crate::value::Value;
use rtcl_parser::ByteCode;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

#[cfg(not(feature = "embedded"))]
use std::collections::HashSet;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HashSet;

#[cfg(not(feature = "embedded"))]
use std::rc::Rc;

#[cfg(feature = "embedded")]
use alloc::rc::Rc;

/// A procedure definition.
#[derive(Debug, Clone)]
pub(crate) struct ProcDef {
    pub params: Vec<(String, Option<String>)>,
    pub body: String,
    /// Static variables: persist across calls. Key = var name, value = current value.
    pub statics: HashMap<String, Value>,
}

/// A link from a local variable name to a variable in another scope.
#[derive(Debug, Clone)]
pub(crate) enum UpvarLink {
    /// Link to `globals[name]`.
    Global(String),
    /// Link to `frames[frame_index].locals[name]`.
    Frame { frame_index: usize, var_name: String },
    /// The link's target was destroyed by a `namespace delete` (or the
    /// containing array was unset) while the alias was live.  Reads keep
    /// working against the alias's seeded local copy; writes error
    /// (tclsh 8.6.17).  `array` distinguishes the two error messages.
    Dead { array: bool, target: String },
}

/// A live `namespace ensemble` — the ensemble command's dispatch table
/// (tclsh 8.6.17).  Keyed in `Interp::ensembles` by the ensemble's
/// registered command name (canonical, `::`-prefixed).
#[derive(Debug, Clone)]
pub(crate) struct EnsembleDef {
    /// Backing namespace the ensemble was created in (configure
    /// -namespace; subcommand implementations resolve here).
    pub namespace: String,
    /// `-map`: subcommand → implementation command prefix (the prefix's
    /// first word is qualified against the backing namespace at set time).
    pub map: Vec<(String, Vec<String>)>,
    /// `-prefixes`: unique-prefix subcommand matching (default on).
    pub prefixes: bool,
    /// `-subcommands`: explicit candidate list overriding the namespace's
    /// exports (None = use the exports).
    pub subcommands: Option<Vec<String>>,
    /// `-unknown`: handler command prefix, stored as typed.
    pub unknown: Option<Vec<String>>,
    /// `-parameters` names — that many leading words are always consumed
    /// as parameter values and prepended to the impl's args.
    pub parameters: Vec<String>,
}

/// A procedure call frame.
#[derive(Debug, Clone)]
pub(crate) struct CallFrame {
    pub locals: HashMap<String, Value>,
    /// Names in `locals` that are arrays (scalar/array distinction).
    pub array_locals: HashSet<String>,
    pub upvars: HashMap<String, UpvarLink>,
    /// Namespace the running proc was defined in (tclsh: `namespace current`
    /// inside the proc resolves here).  Variable *reads* do NOT fall back to
    /// this namespace — only locals, upvar/`variable` links, and `::`-qualified
    /// names are visible in a proc frame.
    pub ns: Option<String>,
    /// Namespace active at CALL time — the context of the caller's script
    /// (e.g. the body of a `namespace eval` this proc was invoked from).
    /// `uplevel` relative levels target this context when the caller is
    /// not a proc frame (tclsh 8.6.17).
    pub call_ns: Option<String>,
    /// Commands created by `local` — deleted when this frame exits.
    pub local_procs: Vec<String>,
    /// Scripts registered by `defer` — executed in reverse order on frame exit.
    pub deferred_scripts: Vec<String>,
    /// Deferred `tailcall` command, armed by the `tailcall` builtin and
    /// fired at frame exit — kept on the frame (not just the completion
    /// error) so a `catch` consuming the completion still lets it fire,
    /// and a second `tailcall` overwrites it (tclsh tailcall-12.3).
    pub tailcall: Option<Vec<String>>,
    /// `info level 0` for this frame: the invocation words as dispatched
    /// (as-typed command name + evaluated arguments), list-rendered.
    pub level0: String,
    /// How many `namespace eval`s were open when this frame was created —
    /// reconstructs the tclsh varFrame chain (proc frames and ns-eval
    /// scopes interleave) for `uplevel` level arithmetic.
    pub ns_depth: usize,
}

/// One live `array startsearch` iteration over an array's element names.
/// A snapshot of the element set plus the array's mutation stamp: any
/// element added or removed (tclsh semantics) invalidates the search.
pub(crate) struct ArraySearch {
    /// Allocation counter value (smallest free at startsearch time).
    pub id: i64,
    /// Rendered identifier, e.g. `s-1-a` — includes the array name as given.
    pub rendered: String,
    /// Remaining element names, in snapshot order.
    pub elements: Vec<String>,
    /// Snapshot of the array's stamp when the search started.
    pub stamp: u64,
}

/// Per-array search table: tclsh's monotonic search counter (reset when the
/// last search goes away) plus the live searches.
pub(crate) struct ArraySearchList {
    pub ctr: i64,
    pub active: Vec<ArraySearch>,
}

/// One registered `trace add variable` callback: canonical ops (sorted
/// array/read/unset/write) plus the script invoked with `name1 name2 op`
/// appended.
#[derive(Clone)]
pub(crate) struct VarTrace {
    pub ops: Vec<String>,
    pub script: String,
}

/// One registered `trace add execution` callback: identity (tclsh's
/// TraceObservation record — `trace remove` matches the *registration*, so a
/// remove+re-add is a different record even if byte-identical), canonical ops
/// (sorted enter/leave/enterstep/leavestep) plus the script invoked with the
/// command text and op appended.
#[derive(Clone)]
pub(crate) struct ExecTrace {
    pub id: u64,
    pub ops: Vec<String>,
    pub script: String,
}

/// Enterstep bookkeeping for one dispatching command (see
/// [`Interp::exec_step_begin`], implemented in `commands::trace`).
pub(crate) struct ExecStepCtx {
    /// Resolved key of the enclosing proc.
    pub key: String,
    /// The command's words, list-rendered (tclsh's substituted text).
    pub cmdtext: String,
    /// Ids of the caller's step records at enterstep time.
    pub snapshot: Vec<u64>,
}

/// Tcl interpreter.
pub struct Interp {
    /// Variables (global scope).
    pub(crate) globals: HashMap<String, Value>,
    /// Names in `globals` that are arrays (scalar/array distinction).
    pub(crate) array_globals: HashSet<String>,
    /// Procedure call frames (empty at global level).
    pub(crate) frames: Vec<CallFrame>,
    /// Commands (built-in and registered).
    pub(crate) commands: HashMap<String, CommandFunc>,
    /// Command category metadata.
    pub(crate) command_categories: HashMap<String, CommandCategory>,
    /// Command metadata (usage + help) for built-in and registered commands.
    pub(crate) command_meta: HashMap<String, CommandMeta>,
    /// User-defined procedures.  `Rc` so per-call dispatch clones a
    /// reference (params + body + statics are NOT copied per call);
    /// statics write-back goes through `Rc::make_mut`.
    pub(crate) procs: HashMap<String, Rc<ProcDef>>,
    /// Parse-tree cache: script text → AST.  Parsing is a pure function
    /// of the text (substitution happens after parse), so a cached tree
    /// is interchangeable with a fresh parse; bodies re-evaluated per
    /// call/iteration (procs, loops) skip re-tokenization.  Entries are
    /// bounded (see `eval`) — wasm32 is a target.
    pub(crate) parse_cache: HashMap<String, Rc<Vec<rtcl_parser::Command>>>,
    /// Call stack depth (for recursion limit).
    pub(crate) call_depth: usize,
    /// Maximum call depth.
    pub(crate) max_call_depth: usize,
    /// Last result.
    pub(crate) result: Value,
    /// Bytecode cache — keyed by script source.
    pub(crate) code_cache: HashMap<String, ByteCode>,
    /// Package registry: name → version string.
    #[cfg(feature = "package")]
    pub(crate) packages: HashMap<String, String>,
    /// Current namespace ("::") at the global level.
    pub(crate) current_namespace: String,
    /// Known namespaces ("::" always present).
    pub(crate) namespaces: HashMap<String, commands::namespace::NamespaceInfo>,
    /// `namespace import` aliases: fully-qualified alias name → the fully
    /// qualified name of the original command.  Aliases stay separate from
    /// `procs` so the origin's *current* body always dispatches (tclsh
    /// semantics: redefining the source proc is visible through the alias).
    pub(crate) import_aliases: HashMap<String, String>,
    /// Per-namespace `namespace unknown` handler scripts (raw, as given).
    pub(crate) ns_unknown: HashMap<String, String>,
    /// Live `namespace ensemble`s keyed by registered command name.
    pub(crate) ensembles: HashMap<String, EnsembleDef>,
    /// Live array searches keyed by the owning scope's stamp key.
    pub(crate) array_searches: HashMap<String, ArraySearchList>,
    /// Mutation counters per array (element-set changes invalidate searches).
    pub(crate) array_stamps: HashMap<String, u64>,
    /// Container-generation counters per array, bumped on array-level
    /// create/destroy only (`array get` uses them to spot a callback that
    /// destroyed and recreated the array — snapshot elements then skip).
    pub(crate) array_generations: HashMap<String, u64>,
    /// Whole-variable traces keyed by the owning scope's stamp key.
    pub(crate) var_traces: HashMap<String, Vec<VarTrace>>,
    /// Element-specific traces: stamp key → element name → traces.
    pub(crate) elem_traces: HashMap<String, HashMap<String, Vec<VarTrace>>>,
    /// Elements that exist only to carry a trace (traced but never set).
    pub(crate) trace_phantoms: HashMap<String, std::collections::HashSet<String>>,
    /// `trace add command` registrations (stored; rename/delete traces).
    pub(crate) cmd_traces: HashMap<String, Vec<(Vec<String>, String)>>,
    /// `trace add execution` registrations (stored; enter/leave/step traces).
    pub(crate) exec_traces: HashMap<String, Vec<ExecTrace>>,
    /// Identity counter for [`ExecTrace`] records.
    pub(crate) exec_trace_ids: u64,
    /// Resolved command keys of the procs currently executing (innermost
    /// last) — execution `enterstep` traces resolve the *caller* command
    /// through this stack.
    pub(crate) exec_step_stack: Vec<String>,
    /// > 0 while an execution-trace callback itself is running (its inner
    /// commands do not re-trigger step traces — tclsh suppresses them).
    pub(crate) exec_step_running: u32,
    /// Eval-level (no call frame) variable aliases created by `upvar` /
    /// `variable`: canonical local flat key → canonical target flat key.
    /// At the global level tclsh's `upvar`/`variable` link two variables;
    /// rtcl's flat global table models that with this redirect.  A Vec so
    /// `namespace eval` can truncate to its entry mark on exit.
    pub(crate) flat_aliases: Vec<(String, String)>,
    /// Eval-level aliases whose target was deleted (namespace removal):
    /// reads keep the redirect, writes error.
    pub(crate) dead_flat: Vec<String>,
    /// Flat keys of variables a `variable` command declared at eval level
    /// (tclsh: the ns-eval varFrame holds a reference, so a namespace that
    /// deletes ITSELF leaves the declared variable readable for the rest
    /// of the body — var-1.16/1.17's `set result` after the self-delete).
    pub(crate) ns_variable_links: Vec<String>,
    /// Namespace variables kept alive past `namespace delete` because a
    /// live variable link still references them; dropped when the
    /// declaring `namespace eval` exits.
    pub(crate) ns_eval_keep: Vec<String>,
    /// Accumulated `errorInfo` of the error currently propagating
    /// (message + `while executing` / `invoked from within` frames).
    /// `None` when no error is in flight; consumed by `catch`.
    pub(crate) err_info: Option<String>,
    /// Suppress the next script-harness frame append — the erroring
    /// command already wrote errorInfo itself (`error msg info`) or
    /// propagates framelessly (`while`/`if` bodies, tclsh's inlined loop
    /// instructions).
    pub(crate) err_fresh: bool,
    /// Set when the raise site of the in-flight error installed
    /// ::errorCode itself (scan formats, exec CHILDSTATUS, `error`,
    /// `return -errorcode`); a `catch` must then preserve it instead of
    /// deriving the code from the error variant.
    pub(crate) err_code_raised: bool,
    /// Line of the last erroring command recorded by a script harness;
    /// consumed by `(procedure ... line N)`, `(in namespace eval ...
    /// script line N)`, `("uplevel" body line N)`, `(file ... line N)`.
    pub(crate) err_line: usize,
    /// Newlines between the enclosing script's first line and the script
    /// currently being evaluated by a construct that re-evals a word of
    /// its own source (`while`/`for`/`foreach` bodies): a body command's
    /// parse-relative line + this offset is the line within the enclosing
    /// script, which is what `(procedure ... line N)` must report (tclsh
    /// keeps one absolute line table).  0 at script level.
    pub(crate) line_offset: usize,
    /// Line of the command currently dispatching (parse-relative).
    pub(crate) cur_cmd_line: usize,
    /// Raw source of each word of the command currently dispatching,
    /// aligned with its arguments — braced/quoted script words carry their
    /// delimiters, letting constructs recover their body's line offset.
    /// `Rc` (shared with the cached parse tree): save/restore per dispatch
    /// is a refcount bump.
    pub(crate) cur_cmd_word_srcs: Rc<Vec<String>>,
    /// Source text of the command currently dispatching, for constructs
    /// that need the raw invocation (`info level 0` inside
    /// `namespace eval`).  `Rc<str>` shared with the parse tree.
    pub(crate) cur_cmd_text: Rc<str>,
    /// `info level 0` inside `namespace eval`: the ns-eval command's
    /// source text, one entry per live `namespace eval` (tclsh's
    /// namespace-eval varFrame is visible to `info level 0`).
    pub(crate) ns_level0: Vec<String>,
    /// The qualified namespace of each live `namespace eval`, parallel to
    /// [`Interp::ns_level0`] — uplevel's scope chain needs the target's
    /// namespace, not just its level-0 text.
    pub(crate) ns_stack: Vec<String>,
    /// Set when a word's error came from a command substitution (the
    /// nested eval already logged its frames; the enclosing command logs
    /// none and marks [`Interp::err_pending_top`] instead).
    pub(crate) err_from_subst: bool,
    /// Source of the outermost command whose word substitution errored,
    /// waiting to see if the error escapes uncaught — the top-level
    /// report then adds `invoked from within "<text>"` (tclsh logs the
    /// enclosing command only when nothing else consumed the log).
    pub(crate) err_pending_top: Option<Rc<str>>,
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
    /// Channel table (stdin/stdout/stderr + opened files/pipes).
    #[cfg(feature = "std")]
    pub channels: crate::channel::ChannelTable,
    /// 宿主控制台服务：无 `io` 特性构建里 `puts` 的 stdout/stderr 落点
    /// （原生 stdio，或 wasm 宿主注入的 JS 实现）。
    pub(crate) console: Box<dyn crate::host::HostConsole>,
    /// Alias definitions (name → target + prefix args).
    pub(crate) aliases: HashMap<String, commands::introspect::AliasInfo>,
    /// Reference table for ref/getref/setref.
    pub(crate) references: HashMap<String, commands::introspect::RefInfo>,
    /// Next reference ID counter.
    pub(crate) next_ref_id: u64,
    /// Saved command definitions for upcall support.
    pub(crate) saved_commands: HashMap<String, commands::introspect::SavedCommand>,
    /// Set of tainted variable names.
    pub(crate) tainted_vars: HashMap<String, bool>,
    /// Execution trace callback script (xtrace). Empty = disabled.
    pub(crate) xtrace_callback: String,
    /// Event queue for `after`/`vwait`/`update`.
    #[cfg(feature = "std")]
    pub(crate) event_queue: Vec<TimedEvent>,
    /// Next event ID counter for `after` scheduling.
    #[cfg(feature = "std")]
    pub(crate) next_event_id: u64,
    /// Child interpreter table for `interp` command.
    #[cfg(feature = "std")]
    pub(crate) child_interps: HashMap<String, Box<Interp>>,
    /// Next child interpreter ID counter.
    #[cfg(feature = "std")]
    pub(crate) next_interp_id: u64,
    /// One-shot level-0 word override for the next `call_proc` frame
    /// (`apply` renders `info level 0` as `apply {<term>} <args...>`,
    /// which the plain arg-list form can't express).
    pub(crate) frame_level0_args: Option<Vec<Value>>,
    /// TclOO object-system state (`oo::` commands, objects, classes).
    pub(crate) oo: commands::oo::OoState,
    /// Reflected channels created by `chan create`, keyed by handle
    /// (`rc0`, `rc1`, ...). Reads/writes route through the handler script.
    #[cfg(feature = "io")]
    pub(crate) reflected: HashMap<String, commands::chan_io::ReflectedChannel>,
    /// Transforms pushed with `chan push`, keyed by the channel handle they
    /// wrap.
    #[cfg(feature = "io")]
    pub(crate) transforms: HashMap<String, commands::chan_io::ChannelTransform>,
}

/// A scheduled time event (for `after ms script`).
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub(crate) struct TimedEvent {
    /// Unique event ID.
    pub id: u64,
    /// Absolute fire time (milliseconds since process start, from `Instant`).
    pub fire_at_ms: u64,
    /// Script to evaluate when the event fires.
    pub script: String,
    /// Whether this is an idle event (`after idle`).
    pub is_idle: bool,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    /// Create a new interpreter.
    pub fn new() -> Self {
        let mut interp = Interp {
            globals: HashMap::new(),
            array_globals: HashSet::new(),
            array_searches: HashMap::new(),
            array_stamps: HashMap::new(),
            array_generations: HashMap::new(),
            var_traces: HashMap::new(),
            elem_traces: HashMap::new(),
            trace_phantoms: HashMap::new(),
            cmd_traces: HashMap::new(),
            exec_traces: HashMap::new(),
            exec_trace_ids: 0,
            exec_step_stack: Vec::new(),
            exec_step_running: 0,
            flat_aliases: Vec::new(),
            dead_flat: Vec::new(),
            ns_variable_links: Vec::new(),
            ns_eval_keep: Vec::new(),
            err_info: None,
            err_fresh: false,
            err_code_raised: false,
            err_line: 1,
            line_offset: 0,
            cur_cmd_line: 0,
            cur_cmd_word_srcs: Rc::new(Vec::new()),
            cur_cmd_text: Rc::from(""),
            err_from_subst: false,
            err_pending_top: None,
            ns_level0: Vec::new(),
            ns_stack: Vec::new(),
            frames: Vec::new(),
            commands: HashMap::new(),
            command_categories: HashMap::new(),
            command_meta: HashMap::new(),
            procs: HashMap::new(),
            parse_cache: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
            code_cache: HashMap::new(),
            #[cfg(feature = "package")]
            packages: HashMap::new(),
            current_namespace: "::".to_string(),
            namespaces: {
                let mut ns = HashMap::new();
                ns.insert("::".to_string(), commands::namespace::NamespaceInfo::default());
                ns
            },
            import_aliases: HashMap::new(),
            ns_unknown: HashMap::new(),
            ensembles: HashMap::new(),
            #[cfg(feature = "std")]
            script_name: String::new(),
            #[cfg(feature = "std")]
            channels: crate::channel::ChannelTable::new(),
            console: {
                #[cfg(feature = "std")]
                let c: Box<dyn crate::host::HostConsole> = Box::new(crate::host::NativeConsole);
                #[cfg(not(feature = "std"))]
                let c: Box<dyn crate::host::HostConsole> = Box::new(crate::host::NullConsole);
                c
            },
            aliases: HashMap::new(),
            references: HashMap::new(),
            next_ref_id: 0,
            saved_commands: HashMap::new(),
            tainted_vars: HashMap::new(),
            xtrace_callback: String::new(),
            #[cfg(feature = "std")]
            event_queue: Vec::new(),
            #[cfg(feature = "std")]
            next_event_id: 1,
            #[cfg(feature = "std")]
            child_interps: HashMap::new(),
            #[cfg(feature = "std")]
            next_interp_id: 1,
            frame_level0_args: None,
            oo: commands::oo::OoState::default(),
            #[cfg(feature = "io")]
            reflected: HashMap::new(),
            #[cfg(feature = "io")]
            transforms: HashMap::new(),
        };
        commands::oo::init(&mut interp);
        interp.register_builtins();
        interp.init_special_vars();
        interp.load_stdlib();
        interp
    }

    /// Load the Tcl-level standard library (embedded at compile time).
    fn load_stdlib(&mut self) {
        const STDLIB_TCL: &str = include_str!("../stdlib.tcl");
        if let Err(e) = self.eval(STDLIB_TCL) {
            panic!("stdlib.tcl failed to load: {e}");
        }
    }

    /// 注入宿主控制台实现（wasm 宿主由此把 `puts` 的输出接到 JS 回调；
    /// 原生构建默认已是 [`crate::host::NativeConsole`]，无需调用）。
    pub fn set_console(&mut self, console: Box<dyn crate::host::HostConsole>) {
        self.console = console;
    }

    /// Populate special global variables (`$env`, `$tcl_platform`, etc.).
    fn init_special_vars(&mut self) {
        // --- $env array (mirror OS environment) ---
        #[cfg(feature = "env")]
        for (key, val) in std::env::vars() {
            let full = format!("env({})", key);
            self.globals.insert(full, Value::from_str(&val));
        }
        #[cfg(feature = "env")]
        if !self.globals.contains_key("env") {
            self.globals.insert("env".to_string(), Value::empty());
        }

        // --- $tcl_platform array ---
        self.globals.insert("tcl_platform(engine)".to_string(), Value::from_str("rtcl"));
        self.globals.insert(
            "tcl_platform(os)".to_string(),
            Value::from_str(std::env::consts::OS),
        );
        self.globals.insert(
            "tcl_platform(platform)".to_string(),
            Value::from_str(if cfg!(unix) { "unix" } else if cfg!(windows) { "windows" } else { "unknown" }),
        );
        self.globals.insert(
            "tcl_platform(machine)".to_string(),
            Value::from_str(std::env::consts::ARCH),
        );
        self.globals.insert(
            "tcl_platform(osVersion)".to_string(),
            Value::from_str(""),
        );
        self.globals.insert(
            "tcl_platform(byteOrder)".to_string(),
            Value::from_str(if cfg!(target_endian = "little") { "littleEndian" } else { "bigEndian" }),
        );
        self.globals.insert(
            "tcl_platform(wordSize)".to_string(),
            Value::from_int(std::mem::size_of::<usize>() as i64),
        );
        self.globals.insert(
            "tcl_platform(pointerSize)".to_string(),
            Value::from_int(std::mem::size_of::<*const ()>() as i64),
        );
        self.globals.insert("tcl_platform".to_string(), Value::empty());
        self.array_globals.insert("tcl_platform".to_string());
        self.array_globals.insert("env".to_string());

        // --- $argv0, $argv, $argc (empty defaults — cli layer overrides) ---
        self.globals.insert("argv0".to_string(), Value::from_str(""));
        self.globals.insert("argv".to_string(), Value::from_str(""));
        self.globals.insert("argc".to_string(), Value::from_int(0));

        // --- $tcl_interactive ---
        self.globals.insert("tcl_interactive".to_string(), Value::from_int(0));

        // --- $errorInfo (empty until an error occurs); $errorCode does
        // not exist until an error sets it (tclsh: info exists → 0) ---
        self.globals.insert("errorInfo".to_string(), Value::from_str(""));

        // --- $auto_path (empty list — package system can populate later) ---
        self.globals.insert("auto_path".to_string(), Value::from_str(""));

        // --- $tcl_version, $tcl_patchLevel ---
        self.globals.insert("tcl_version".to_string(), Value::from_str("8.6"));
        self.globals.insert("tcl_patchLevel".to_string(), Value::from_str("8.6.0-rtcl"));
        self.globals.insert("tcl_library".to_string(), Value::from_str("/usr/share/tcltk/tcl8.6"));
    }

    /// Whether we are inside a procedure scope.
    #[allow(dead_code)]
    pub(crate) fn in_proc(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Reference to variable storage for the current scope (for iteration).
    /// Does NOT follow upvar links.
    pub(crate) fn scope_vars(&self) -> &HashMap<String, Value> {
        if let Some(frame) = self.frames.last() {
            &frame.locals
        } else {
            &self.globals
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_var() {
        let mut interp = Interp::new();
        interp.eval("set x 42").unwrap();
        assert_eq!(interp.get_var("x").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_expr() {
        let mut interp = Interp::new();
        let result = interp.eval("expr 1 + 2").unwrap();
        assert_eq!(result.as_int(), Some(3));
    }

    #[test]
    fn test_while() {
        let mut interp = Interp::new();
        interp.eval("set i 0").unwrap();
        interp.eval("while {$i < 5} { incr i }").unwrap();
        assert_eq!(interp.get_var("i").unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_tailcall_factorial() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc fact {n acc} {
                    if {$n <= 1} { return $acc }
                    tailcall fact [expr {$n - 1}] [expr {$n * $acc}]
                }",
            )
            .unwrap();
        let result = interp.eval("fact 10 1").unwrap();
        assert_eq!(result.as_str(), "3628800");
    }

    #[test]
    fn test_tailcall_deep_no_overflow() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc countdown {n} {
                    if {$n <= 0} { return done }
                    tailcall countdown [expr {$n - 1}]
                }",
            )
            .unwrap();
        // Without TCO this would overflow the 1000-deep call stack
        let result = interp.eval("countdown 5000").unwrap();
        assert_eq!(result.as_str(), "done");
    }

    #[test]
    fn test_tailcall_mutual_recursion() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc tc_even {n} {
                    if {$n == 0} { return 1 }
                    tailcall tc_odd [expr {$n - 1}]
                }
                proc tc_odd {n} {
                    if {$n == 0} { return 0 }
                    tailcall tc_even [expr {$n - 1}]
                }",
            )
            .unwrap();
        let result = interp.eval("tc_even 100").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // --- stdlib.tcl tests ---

    #[test]
    fn test_stdlib_throw_error() {
        let mut interp = Interp::new();
        let err = interp.eval("throw error {something broke}").unwrap_err();
        assert!(err.to_string().contains("something broke"));
    }

    #[test]
    fn test_stdlib_throw_ok() {
        let mut interp = Interp::new();
        let result = interp.eval("throw ok hello").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    #[test]
    fn test_stdlib_throw_catch() {
        let mut interp = Interp::new();
        let result = interp
            .eval("catch {throw error oops} msg; set msg")
            .unwrap();
        assert_eq!(result.as_str(), "oops");
    }

    #[test]
    fn test_stdlib_parray() {
        let mut interp = Interp::new();
        // parray writes to stdout; just verify it doesn't error
        interp.eval("array set x {a 1 b 2 c 3}").unwrap();
        interp.eval("parray x").unwrap();
    }

    // --- auto_qualify (stdlib.tcl, tclsh 8.6 init.tcl parity / gen_init) ---

    #[test]
    fn test_auto_qualify_manpage_examples() {
        let mut interp = Interp::new();
        let cases = [
            ("::foo::bar ::blue", "::foo::bar"),
            ("::global ::sub", "global"),
            ("nocolons ::", "nocolons"),
            ("nocolons ::sub", "::sub::nocolons nocolons"),
            ("foo::bar ::", "::foo::bar"),
            ("foo::bar ::sub", "::sub::foo::bar ::foo::bar"),
            (":::foo::::bar ::blue", "::foo::bar"),
            (":::foo ::bar", "foo"),
        ];
        for (call, want) in cases {
            let got = interp
                .eval(&format!("auto_qualify {call}"))
                .unwrap()
                .as_str()
                .to_string();
            assert_eq!(got, want, "auto_qualify {call}");
        }
    }

    // --- ::tcl::pkgconfig (misc.rs, tclPkgConfig.c parity / gen_config) ---

    #[test]
    fn test_pkgconfig_surface() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("llength [::tcl::pkgconfig list]")
                .unwrap()
                .as_str(),
            "18"
        );
        assert_eq!(
            interp
                .eval("::tcl::pkgconfig get bindir,install")
                .unwrap()
                .as_str(),
            "/usr/bin"
        );
        assert_eq!(
            interp
                .eval("catch {::tcl::pkgconfig get foo} m; set ::errorCode")
                .unwrap()
                .as_str(),
            "TCL LOOKUP CONFIG foo"
        );
    }

    #[test]
    fn test_pkgconfig_arity_and_subcommand_errors() {
        let mut interp = Interp::new();
        let cases = [
            (
                "catch {::tcl::pkgconfig} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig subcommand ?arg?\"",
            ),
            (
                "catch {::tcl::pkgconfig foo} m; set m",
                "bad subcommand \"foo\": must be get or list",
            ),
            (
                "catch {::tcl::pkgconfig foo} m; set ::errorCode",
                "TCL LOOKUP INDEX subcommand foo",
            ),
            (
                "catch {::tcl::pkgconfig list foo} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig list\"",
            ),
            (
                "catch {::tcl::pkgconfig get} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig get key\"",
            ),
            (
                "catch {::tcl::pkgconfig get foo bar} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig subcommand ?arg?\"",
            ),
            // Prefix resolution + ambiguity (Tcl_GetIndexFromObj semantics).
            ("llength [::tcl::pkgconfig l]", "18"),
            (
                "catch {::tcl::pkgconfig \"\"} m; set m",
                "ambiguous subcommand \"\": must be get or list",
            ),
            (
                "catch {::tcl::pkgconfig \"\"} m; set ::errorCode",
                "TCL LOOKUP INDEX subcommand {}",
            ),
        ];
        for (script, want) in cases {
            let got = interp.eval(script).unwrap().as_str().to_string();
            assert_eq!(got, want, "script: {script}");
        }
    }

    // --- Phase 5B stdlib tests ---

    #[test]
    fn test_stdlib_function() {
        let mut interp = Interp::new();
        let result = interp.eval("function hello").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    #[test]
    fn test_stdlib_lambda() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set f [lambda {x} { expr {$x * 2} }]
            $f 21
        "#).unwrap();
        assert_eq!(result.as_str(), "42");
    }

    #[test]
    fn test_stdlib_curry() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set add5 [curry expr 5 +]
            $add5 10
        "#).unwrap();
        assert_eq!(result.as_str(), "15");
    }

    #[test]
    fn test_stdlib_loop() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set sum 0
            loop i 1 6 {
                incr sum $i
            }
        "#).unwrap();
        let result = interp.eval("set sum").unwrap();
        assert_eq!(result.as_str(), "15");
    }

    #[test]
    fn test_stdlib_loop_with_step() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set vals {}
            loop i 0 10 2 {
                lappend vals $i
            }
        "#).unwrap();
        let result = interp.eval("set vals").unwrap();
        assert_eq!(result.as_str(), "0 2 4 6 8");
    }

    #[test]
    fn test_stdlib_dict_getdef() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set d [dict create a 1 b 2]
            dict getdef $d c 99
        "#).unwrap();
        assert_eq!(result.as_str(), "99");
    }

    #[test]
    fn test_stdlib_dict_getdef_exists() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set d [dict create a 1 b 2]
            dict getdef $d a 99
        "#).unwrap();
        assert_eq!(result.as_str(), "1");
    }

    #[test]
    fn test_stdlib_ensemble() {
        let mut interp = Interp::new();
        interp.eval(r#"
            proc {myns add} {a b} { expr {$a + $b} }
            proc {myns mul} {a b} { expr {$a * $b} }
            ensemble myns
        "#).unwrap();
        let result = interp.eval("myns add 3 4").unwrap();
        assert_eq!(result.as_str(), "7");
        let result = interp.eval("myns mul 3 4").unwrap();
        assert_eq!(result.as_str(), "12");
    }

    #[test]
    fn test_stdlib_fileevent_shim() {
        let mut interp = Interp::new();
        // fileevent is a shim that just tailcalls its args
        // We just verify it doesn't error when called with a known command
        let result = interp.eval("fileevent set x 42").unwrap();
        assert_eq!(result.as_str(), "42");
    }

    #[test]
    fn test_stdlib_json_encode_string() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"json::encode "hello world""#).unwrap();
        assert_eq!(result.as_str(), "\"hello world\"");
    }

    #[test]
    fn test_stdlib_json_encode_num() {
        let mut interp = Interp::new();
        let result = interp.eval("json::encode 42 num").unwrap();
        assert_eq!(result.as_str(), "42");
    }

    #[test]
    fn test_stdlib_error_info() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"errorInfo "something failed""#).unwrap();
        assert!(result.as_str().contains("something failed"));
    }

    #[test]
    fn test_stdlib_namespace_inscope() {
        let mut interp = Interp::new();
        interp.eval(r#"
            namespace eval foo {
                proc bar {} { return "in foo" }
            }
        "#).unwrap();
        let result = interp.eval("namespace inscope foo bar").unwrap();
        assert_eq!(result.as_str(), "in foo");
    }

    // --- Variable names with special characters ---

    #[test]
    fn test_var_braced_path_slash() {
        let mut interp = Interp::new();
        interp.eval(r#"set {path/file.exe} "hello""#).unwrap();
        let result = interp.eval("set {path/file.exe}").unwrap();
        assert_eq!(result.as_str(), "hello");
        // Also verify ${} deref syntax in a proc
        interp.eval("proc getit {} { global {path/file.exe}; set x ${path/file.exe} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    #[test]
    fn test_var_braced_absolute_path() {
        let mut interp = Interp::new();
        interp.eval(r#"set {/usr/local/bin/prog} "world""#).unwrap();
        interp.eval("proc getit {} { global {/usr/local/bin/prog}; set x ${/usr/local/bin/prog} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "world");
    }

    #[test]
    fn test_var_braced_dots() {
        let mut interp = Interp::new();
        interp.eval(r#"set {config.server.host} "localhost""#).unwrap();
        interp.eval("proc getit {} { global {config.server.host}; set x ${config.server.host} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "localhost");
    }

    #[test]
    fn test_var_bare_dot_boundary() {
        let mut interp = Interp::new();
        // $foo.bar should be $foo + ".bar", not variable "foo.bar"
        interp.eval("set foo test").unwrap();
        interp.eval("proc getit {} { global foo; set x $foo.bar }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "test.bar");
    }

    #[test]
    fn test_var_braced_with_suffix() {
        let mut interp = Interp::new();
        interp.eval(r#"set {a.b} "test""#).unwrap();
        interp.eval("proc getit {} { global {a.b}; set x ${a.b}.x }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "test.x");
    }

    #[test]
    fn test_var_braced_in_string() {
        let mut interp = Interp::new();
        interp.eval(r#"set {path/file.exe} "/bin/ls""#).unwrap();
        interp.eval(r#"proc getit {} { global {path/file.exe}; set x "exe=${path/file.exe}" }"#).unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "exe=/bin/ls");
    }

    #[test]
    fn test_var_array_dot_slash_key() {
        let mut interp = Interp::new();
        interp.eval(r#"set arr(a.b/c) "value""#).unwrap();
        let result = interp.eval("set arr(a.b/c)").unwrap();
        assert_eq!(result.as_str(), "value");
    }

    #[test]
    fn test_stdlib_defer() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set log {}
            proc cleanup {} {
                global log
                defer {global log; lappend log "deferred1"}
                defer {global log; lappend log "deferred2"}
                lappend log "body"
            }
            cleanup
        "#).unwrap();
        let result = interp.eval("set log").unwrap();
        // defer runs in reverse order on proc exit
        assert_eq!(result.as_str(), "body deferred2 deferred1");
    }
}
