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
mod registry;
mod vars;
mod eval;
mod call;
mod vm_bridge;
mod util;

// Re-export utilities so command modules can reach them via `super::super::glob_match`
pub(crate) use util::{split_array_ref, glob_match};

use crate::command::{CommandFunc, CommandCategory};
use crate::value::Value;
use rtcl_parser::ByteCode;

#[cfg(feature = "std")]
use std::collections::HashMap;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::collections::BTreeMap as HashMap;

/// A procedure definition.
#[derive(Debug, Clone)]
pub(crate) struct ProcDef {
    pub params: Vec<(String, Option<String>)>,
    pub body: String,
}

/// A link from a local variable name to a variable in another scope.
#[derive(Debug, Clone)]
pub(crate) enum UpvarLink {
    /// Link to `globals[name]`.
    Global(String),
    /// Link to `frames[frame_index].locals[name]`.
    Frame { frame_index: usize, var_name: String },
}

/// A procedure call frame.
#[derive(Debug, Clone)]
pub(crate) struct CallFrame {
    pub locals: HashMap<String, Value>,
    pub upvars: HashMap<String, UpvarLink>,
    /// Commands created by `local` — deleted when this frame exits.
    pub local_procs: Vec<String>,
    /// Scripts registered by `defer` — executed in reverse order on frame exit.
    pub deferred_scripts: Vec<String>,
}

/// Tcl interpreter.
pub struct Interp {
    /// Variables (global scope).
    pub(crate) globals: HashMap<String, Value>,
    /// Procedure call frames (empty at global level).
    pub(crate) frames: Vec<CallFrame>,
    /// Commands (built-in and registered).
    pub(crate) commands: HashMap<String, CommandFunc>,
    /// Command category metadata.
    pub(crate) command_categories: HashMap<String, CommandCategory>,
    /// User-defined procedures.
    pub(crate) procs: HashMap<String, ProcDef>,
    /// Call stack depth (for recursion limit).
    pub(crate) call_depth: usize,
    /// Maximum call depth.
    pub(crate) max_call_depth: usize,
    /// Last result.
    pub(crate) result: Value,
    /// Bytecode cache — keyed by script source.
    pub(crate) code_cache: HashMap<String, ByteCode>,
    /// Package registry: name → version string.
    pub(crate) packages: HashMap<String, String>,
    /// Current namespace ("::") at the global level.
    pub(crate) current_namespace: String,
    /// Known namespaces ("::" always present).
    pub(crate) namespaces: HashMap<String, commands::namespace::NamespaceInfo>,
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
    /// Channel table (stdin/stdout/stderr + opened files/pipes).
    #[cfg(feature = "std")]
    pub channels: crate::channel::ChannelTable,
    /// Alias definitions (name → target + prefix args).
    pub(crate) aliases: HashMap<String, commands::introspect::AliasInfo>,
    /// Reference table for ref/getref/setref.
    pub(crate) references: HashMap<String, commands::introspect::RefInfo>,
    /// Next reference ID counter.
    pub(crate) next_ref_id: u64,
    /// Saved command definitions for upcall support.
    pub(crate) saved_commands: HashMap<String, commands::introspect::SavedCommand>,
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
            frames: Vec::new(),
            commands: HashMap::new(),
            command_categories: HashMap::new(),
            procs: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
            code_cache: HashMap::new(),
            packages: HashMap::new(),
            current_namespace: "::".to_string(),
            namespaces: {
                let mut ns = HashMap::new();
                ns.insert("::".to_string(), commands::namespace::NamespaceInfo::default());
                ns
            },
            #[cfg(feature = "std")]
            script_name: String::new(),
            #[cfg(feature = "std")]
            channels: crate::channel::ChannelTable::new(),
            aliases: HashMap::new(),
            references: HashMap::new(),
            next_ref_id: 0,
            saved_commands: HashMap::new(),
        };
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

    /// Populate special global variables (`$env`, `$tcl_platform`, etc.).
    fn init_special_vars(&mut self) {
        // --- $env array (mirror OS environment) ---
        #[cfg(feature = "std")]
        for (key, val) in std::env::vars() {
            let full = format!("env({})", key);
            self.globals.insert(full, Value::from_str(&val));
        }
        #[cfg(feature = "std")]
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

        // --- $argv0, $argv, $argc (empty defaults — cli layer overrides) ---
        self.globals.insert("argv0".to_string(), Value::from_str(""));
        self.globals.insert("argv".to_string(), Value::from_str(""));
        self.globals.insert("argc".to_string(), Value::from_int(0));

        // --- $tcl_interactive ---
        self.globals.insert("tcl_interactive".to_string(), Value::from_int(0));

        // --- $errorCode, $errorInfo (empty until an error occurs) ---
        self.globals.insert("errorCode".to_string(), Value::from_str("NONE"));
        self.globals.insert("errorInfo".to_string(), Value::from_str(""));

        // --- $auto_path (empty list — package system can populate later) ---
        self.globals.insert("auto_path".to_string(), Value::from_str(""));

        // --- $tcl_version, $tcl_patchLevel ---
        self.globals.insert("tcl_version".to_string(), Value::from_str("8.6"));
        self.globals.insert("tcl_patchLevel".to_string(), Value::from_str("8.6.0-rtcl"));
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
