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
    pub(crate) channels: crate::channel::ChannelTable,
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
        };
        interp.register_builtins();
        interp.init_special_vars();
        interp
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
}
