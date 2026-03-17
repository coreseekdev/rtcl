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

/// Tcl interpreter.
pub struct Interp {
    /// Variables (global scope).
    pub(crate) vars: HashMap<String, Value>,
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
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
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
            vars: HashMap::new(),
            commands: HashMap::new(),
            command_categories: HashMap::new(),
            procs: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
            code_cache: HashMap::new(),
            #[cfg(feature = "std")]
            script_name: String::new(),
        };
        interp.register_builtins();
        interp
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
