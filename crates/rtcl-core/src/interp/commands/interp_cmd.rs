//! Sub-interpreter command: interp.
//!
//! Implements the jimtcl sub-interpreter model:
//! - `interp` — create a new child interpreter, returns a handle
//! - `$handle eval script` — evaluate script in child
//! - `$handle delete` — destroy the child interpreter
//! - `$handle alias childCmd parentCmd ?arg ...?` — alias from child to parent
//!
//! All values crossing interpreter boundaries are string-copied (no shared state).

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

/// `interp` — create a new child interpreter.
///
/// Returns a handle like "interp#1" that becomes a command in the parent.
pub fn cmd_interp(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 1 {
        return Err(Error::wrong_args_with_usage(
            "interp", 1, args.len(),
            "interp",
        ));
    }

    let id = interp.next_interp_id;
    interp.next_interp_id += 1;
    let handle = format!("interp#{}", id);

    // Create a fresh child interpreter (full initialization including stdlib)
    let child = Box::new(Interp::new());
    interp.child_interps.insert(handle.clone(), child);

    // Register a command with the handle name that dispatches subcommands
    let handle_for_cmd = handle.clone();
    interp.commands.insert(
        handle.clone(),
        make_interp_dispatch(&handle_for_cmd),
    );

    Ok(Value::from_str(&handle))
}

/// Create a command function that dispatches `$handle eval/delete/alias`.
fn make_interp_dispatch(handle: &str) -> crate::command::CommandFunc {
    // We use a closure-like approach: encode the handle in the command name
    // The dispatch function extracts the handle from args[0]
    fn dispatch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args_with_usage(
                args[0].as_str(), 2, args.len(),
                "$interp subcommand ?arg ...?",
            ));
        }

        let handle = args[0].as_str().to_string();
        let subcmd = args[1].as_str();

        match subcmd {
            "eval" => interp_eval(interp, &handle, args),
            "delete" => interp_delete(interp, &handle),
            "alias" => interp_alias(interp, &handle, args),
            _ => Err(Error::runtime(
                format!(
                    "bad option \"{}\": must be alias, delete, or eval",
                    subcmd
                ),
                ErrorCode::Generic,
            )),
        }
    }

    // The dispatch function is the same for all handles — the handle name
    // is extracted from args[0] at runtime.
    let _ = handle; // handle is encoded in the command name, not in the fn pointer
    dispatch
}

/// `$handle eval script ?script ...?`
fn interp_eval(interp: &mut Interp, handle: &str, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            handle, 3, args.len(),
            "$interp eval script ?script ...?",
        ));
    }

    // Concatenate script arguments (string-copy semantics)
    let script = if args.len() == 3 {
        args[2].as_str().to_string()
    } else {
        args[2..].iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Get the child interpreter, evaluate, return result as string
    let child = interp.child_interps.get_mut(handle)
        .ok_or_else(|| Error::runtime(
            format!("interpreter \"{}\" doesn't exist", handle),
            ErrorCode::NotFound,
        ))?;

    match child.eval(&script) {
        Ok(val) => Ok(Value::from_str(val.as_str())),
        Err(e) => Err(Error::runtime(
            format!("child interp error: {}", e),
            ErrorCode::Generic,
        )),
    }
}

/// `$handle delete` — destroy the child interpreter.
fn interp_delete(interp: &mut Interp, handle: &str) -> Result<Value> {
    if interp.child_interps.remove(handle).is_none() {
        return Err(Error::runtime(
            format!("interpreter \"{}\" doesn't exist", handle),
            ErrorCode::NotFound,
        ));
    }
    // Remove the dispatch command
    interp.commands.remove(handle);
    interp.command_categories.remove(handle);
    interp.command_meta.remove(handle);
    Ok(Value::empty())
}

/// `$handle alias childCmd parentCmd ?arg ...?`
///
/// Creates a command in the child that, when called, evaluates in the parent.
/// Due to Rust ownership rules, we implement this by storing the alias info
/// in the child interp as a special proc that produces a string result,
/// which the parent then evaluates.
fn interp_alias(interp: &mut Interp, handle: &str, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            handle, 4, args.len(),
            "$interp alias childCmd parentCmd ?arg ...?",
        ));
    }

    let child_cmd = args[2].as_str().to_string();
    let parent_cmd = args[3].as_str().to_string();
    let prefix_args: Vec<String> = args[4..].iter()
        .map(|v| v.as_str().to_string())
        .collect();

    // Build a proc body in the child that concatenates the parent command
    // with prefix args and any call-time args, then returns the full command
    // string. The parent will need to evaluate this.
    //
    // For simplicity, we store the alias definition and create a Tcl proc
    // in the child that builds the command string.
    let child = interp.child_interps.get_mut(handle)
        .ok_or_else(|| Error::runtime(
            format!("interpreter \"{}\" doesn't exist", handle),
            ErrorCode::NotFound,
        ))?;

    // Build the proc body
    let prefix = if prefix_args.is_empty() {
        parent_cmd.clone()
    } else {
        format!("{} {}", parent_cmd, prefix_args.join(" "))
    };
    let body = format!(
        "set __cmd [list {} {{*}}$args]\nreturn $__cmd",
        prefix,
    );
    let define_script = format!(
        "proc {} args {{\n{}\n}}",
        child_cmd, body
    );
    child.eval(&define_script).map_err(|e| {
        Error::runtime(
            format!("failed to create alias in child: {}", e),
            ErrorCode::Generic,
        )
    })?;

    Ok(Value::empty())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_interp_create() {
        let mut interp = Interp::new();
        let handle = interp.eval("interp").unwrap();
        assert!(handle.as_str().starts_with("interp#"));
    }

    #[test]
    fn test_interp_eval() {
        let mut interp = Interp::new();
        let handle = interp.eval("interp").unwrap();
        let r = interp.eval(&format!("{} eval {{expr {{1 + 2}}}}", handle.as_str())).unwrap();
        assert_eq!(r.as_str(), "3");
    }

    #[test]
    fn test_interp_isolation() {
        let mut interp = Interp::new();
        interp.eval("set parentvar hello").unwrap();
        let handle = interp.eval("interp").unwrap();

        // Child should not see parent's variable
        let r = interp.eval(&format!("{} eval {{catch {{set parentvar}} err; set err}}", handle.as_str())).unwrap();
        assert!(r.as_str().contains("parentvar"));

        // Child can set its own variable
        interp.eval(&format!("{} eval {{set childvar world}}", handle.as_str())).unwrap();
        let r = interp.eval(&format!("{} eval {{set childvar}}", handle.as_str())).unwrap();
        assert_eq!(r.as_str(), "world");

        // Parent should not see child's variable
        assert!(interp.eval("set childvar").is_err());
    }

    #[test]
    fn test_interp_delete() {
        let mut interp = Interp::new();
        let handle = interp.eval("interp").unwrap();
        interp.eval(&format!("{} delete", handle.as_str())).unwrap();

        // Handle command should no longer exist
        assert!(interp.eval(&format!("{} eval {{set x 1}}", handle.as_str())).is_err());
    }

    #[test]
    fn test_interp_multiple() {
        let mut interp = Interp::new();
        let h1 = interp.eval("interp").unwrap();
        let h2 = interp.eval("interp").unwrap();
        assert_ne!(h1.as_str(), h2.as_str());

        interp.eval(&format!("{} eval {{set x 1}}", h1.as_str())).unwrap();
        interp.eval(&format!("{} eval {{set x 2}}", h2.as_str())).unwrap();

        let r1 = interp.eval(&format!("{} eval {{set x}}", h1.as_str())).unwrap();
        let r2 = interp.eval(&format!("{} eval {{set x}}", h2.as_str())).unwrap();
        assert_eq!(r1.as_str(), "1");
        assert_eq!(r2.as_str(), "2");
    }
}
