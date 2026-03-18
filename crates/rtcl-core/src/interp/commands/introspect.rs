//! Interpreter introspection commands: exists, alias, local, upcall, unknown, ref/getref/setref/finalize, stacktrace.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

// ---------- exists ----------

/// exists ?-var|-proc|-command|-alias? name
pub fn cmd_exists(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "exists", 2, args.len(), "?-var|-proc|-command|-alias? name",
        ));
    }

    if args.len() == 2 {
        // No qualifier — check in order: command > proc > variable
        let name = args[1].as_str();
        let found = interp.commands.contains_key(name)
            || interp.procs.contains_key(name)
            || interp.var_exists(name);
        return Ok(Value::from_bool(found));
    }

    let qualifier = args[1].as_str();
    let name = args[2].as_str();
    match qualifier {
        "-var" | "-variable" => Ok(Value::from_bool(interp.var_exists(name))),
        "-proc" => Ok(Value::from_bool(interp.procs.contains_key(name))),
        "-command" | "-cmd" => {
            let found = interp.commands.contains_key(name) || interp.procs.contains_key(name);
            Ok(Value::from_bool(found))
        }
        "-alias" => Ok(Value::from_bool(interp.aliases.contains_key(name))),
        #[cfg(feature = "std")]
        "-channel" => Ok(Value::from_bool(interp.channels.contains(name))),
        _ => Err(Error::runtime(
            format!("bad option \"{}\": must be -var, -proc, -command, -alias, or -channel", qualifier),
            ErrorCode::Generic,
        )),
    }
}

// ---------- alias ----------

/// alias newname cmd ?arg ...?
pub fn cmd_alias(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("alias", 3, args.len(), "newname cmd ?arg ...?"));
    }
    let new_name = args[1].as_str().to_string();
    let target_cmd = args[2].as_str().to_string();
    let prefix_args: Vec<String> = args[3..].iter().map(|a| a.as_str().to_string()).collect();

    // Store the alias definition
    interp.aliases.insert(new_name.clone(), AliasInfo {
        target: target_cmd,
        prefix_args,
    });

    // Register a dispatcher command in the command table
    interp.commands.insert(new_name.clone(), alias_dispatch);
    interp.command_categories.insert(new_name, crate::command::CommandCategory::Extension);
    Ok(Value::empty())
}

/// Alias dispatch function — looks up the alias definition and calls the target.
fn alias_dispatch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let name = args[0].as_str();
    let info = interp.aliases.get(name).cloned().ok_or_else(|| {
        Error::runtime(format!("alias \"{}\" not found", name), ErrorCode::NotFound)
    })?;

    // Build the full argument list: target_cmd prefix_args... caller_args...
    let mut full_args = Vec::with_capacity(1 + info.prefix_args.len() + args.len() - 1);
    full_args.push(Value::from_str(&info.target));
    for a in &info.prefix_args {
        full_args.push(Value::from_str(a));
    }
    for a in &args[1..] {
        full_args.push(a.clone());
    }

    // Look up target: first check procs, then builtins
    if let Some(proc_def) = interp.procs.get(&info.target).cloned() {
        interp.call_proc(&proc_def, &full_args)
    } else if let Some(f) = interp.commands.get(&info.target).cloned() {
        f(interp, &full_args)
    } else {
        Err(Error::invalid_command(&info.target))
    }
}

// ---------- local ----------

/// local cmd ?args?
/// Marks a command (usually a proc created by lambda/curry) for deletion when
/// the current call frame exits.
pub fn cmd_local(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("local", 2, args.len(), "cmd ?arg ...?"));
    }

    // First, evaluate the command (which typically creates the proc)
    let result = {
        let cmd_args: Vec<Value> = args[1..].to_vec();
        let cmd_name = cmd_args[0].as_str();

        if let Some(proc_def) = interp.procs.get(cmd_name).cloned() {
            interp.call_proc(&proc_def, &cmd_args)?
        } else if let Some(f) = interp.commands.get(cmd_name).cloned() {
            f(interp, &cmd_args)?
        } else {
            return Err(Error::invalid_command(cmd_name));
        }
    };

    // Mark the resulting proc name for cleanup on frame exit
    let proc_name = result.as_str().to_string();
    if !proc_name.is_empty() {
        if let Some(frame) = interp.frames.last_mut() {
            frame.local_procs.push(proc_name);
        }
    }

    Ok(result)
}

// ---------- upcall ----------

/// upcall cmd ?args?
/// Calls the previous definition of a command that was overridden with `local`.
pub fn cmd_upcall(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("upcall", 2, args.len(), "cmd ?arg ...?"));
    }
    let name = args[1].as_str();

    // Look for the saved upcall target
    let saved = interp.saved_commands.get(name).cloned();
    match saved {
        Some(SavedCommand::Proc(proc_def)) => {
            interp.call_proc(&proc_def, args)
        }
        Some(SavedCommand::Builtin(f)) => f(interp, args),
        None => Err(Error::runtime(
            format!("no previous command definition for \"{}\"", name),
            ErrorCode::NotFound,
        )),
    }
}

// ---------- unknown handler ----------

/// unknown args... (the handler proc; this registers the mechanism)
/// This isn't a standalone command — it modifies eval_command dispatch.
/// The actual "unknown" proc is user-defined in Tcl.
/// We provide a default stub that errors.
pub fn cmd_unknown(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Default unknown handler: just error
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("unknown", 2, args.len(), "cmdName ?arg ...?"));
    }
    Err(Error::invalid_command(args[1].as_str()))
}

// ---------- defer ----------

/// defer script
/// Register a script to be executed when the current proc exits.
pub fn cmd_defer(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("defer", 2, args.len(), "script"));
    }
    let script = args[1].as_str().to_string();
    if let Some(frame) = interp.frames.last_mut() {
        frame.deferred_scripts.push(script);
    }
    Ok(Value::empty())
}

// ---------- ref / getref / setref / finalize ----------

/// ref value ?tag? ?finalizer?
/// Creates a reference (opaque handle) to a value.
pub fn cmd_ref(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage("ref", 2, args.len(), "value ?tag? ?finalizer?"));
    }
    let value = args[1].clone();
    let tag = if args.len() > 2 { args[2].as_str().to_string() } else { String::new() };
    let finalizer = if args.len() > 3 { Some(args[3].as_str().to_string()) } else { None };

    let id = interp.next_ref_id;
    interp.next_ref_id += 1;

    let handle = format!("<reference.{}.{:08x}>", tag, id);
    interp.references.insert(handle.clone(), RefInfo { value, tag, finalizer });
    Ok(Value::from_str(&handle))
}

/// getref reference
pub fn cmd_getref(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("getref", 2, args.len(), "reference"));
    }
    let handle = args[1].as_str();
    let info = interp.references.get(handle).ok_or_else(|| {
        Error::runtime(format!("invalid reference \"{}\"", handle), ErrorCode::NotFound)
    })?;
    Ok(info.value.clone())
}

/// setref reference value
pub fn cmd_setref(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage("setref", 3, args.len(), "reference newValue"));
    }
    let handle = args[1].as_str().to_string();
    let new_value = args[2].clone();
    let info = interp.references.get_mut(&handle).ok_or_else(|| {
        Error::runtime(format!("invalid reference \"{}\"", handle), ErrorCode::NotFound)
    })?;
    let old_value = std::mem::replace(&mut info.value, new_value);
    Ok(old_value)
}

/// collect — Run finalizers on unreferenced values.
/// In rtcl we don't have a GC, but we provide the command for compatibility.
pub fn cmd_collect(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 1 {
        return Err(Error::wrong_args("collect", 1, args.len()));
    }
    // Simple mark-and-sweep: find references that aren't stored in any variable
    // For now, just return 0 (no collection needed)
    let _ = interp;
    Ok(Value::from_int(0))
}

/// finalize reference ?destructor?
pub fn cmd_finalize(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("finalize", 2, args.len(), "reference ?destructor?"));
    }
    let handle = args[1].as_str().to_string();
    let info = interp.references.get_mut(&handle).ok_or_else(|| {
        Error::runtime(format!("invalid reference \"{}\"", handle), ErrorCode::NotFound)
    })?;
    if args.len() == 3 {
        // Set new finalizer
        info.finalizer = Some(args[2].as_str().to_string());
        Ok(Value::empty())
    } else {
        // Return current finalizer
        Ok(Value::from_str(info.finalizer.as_deref().unwrap_or("")))
    }
}

// ---------- stacktrace ----------

/// stacktrace — return the current call stack as a list.
/// Each entry is {proc file line}.
pub fn cmd_stacktrace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 1 {
        return Err(Error::wrong_args("stacktrace", 1, args.len()));
    }
    // Build a list of stack entries
    let depth = interp.frames.len();
    let mut entries = Vec::new();
    for i in (0..depth).rev() {
        let frame_info = format!("frame{}", i);
        entries.push(Value::from_str(&frame_info));
        entries.push(Value::from_str(""));  // file (not tracked yet)
        entries.push(Value::from_int(0));    // line (not tracked yet)
    }
    Ok(Value::from_list(&entries))
}

// ---------- pack / unpack ----------

/// pack varName value type bitwidth ?bitoffset?
pub fn cmd_pack(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 5 || args.len() > 6 {
        return Err(Error::wrong_args_with_usage(
            "pack", 5, args.len(), "varName value type bitwidth ?bitoffset?",
        ));
    }
    let var_name = args[1].as_str();
    let value = args[2].as_int().ok_or_else(|| {
        Error::runtime(
            format!("expected integer but got \"{}\"", args[2].as_str()),
            ErrorCode::Generic,
        )
    })?;
    let type_name = args[3].as_str();
    let bit_width = args[4].as_int().ok_or_else(|| {
        Error::runtime(
            format!("expected integer but got \"{}\"", args[4].as_str()),
            ErrorCode::Generic,
        )
    })? as usize;
    let bit_offset = if args.len() == 6 {
        args[5].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[5].as_str()),
                ErrorCode::Generic,
            )
        })? as usize
    } else {
        0
    };

    // Get existing binary data or create empty
    let existing = match interp.get_var(var_name) {
        Ok(v) => v.as_str().as_bytes().to_vec(),
        Err(_) => Vec::new(),
    };

    let total_bytes = (bit_offset + bit_width).div_ceil(8);
    let mut data = existing;
    if data.len() < total_bytes {
        data.resize(total_bytes, 0);
    }

    let is_big_endian = match type_name {
        "be" | "bigendian" => true,
        "le" | "littleendian" | "" => false,
        _ => {
            return Err(Error::runtime(
                format!("unknown type \"{}\": must be le, be, littleendian, or bigendian", type_name),
                ErrorCode::Generic,
            ));
        }
    };

    // Pack the value into the byte array
    pack_int(&mut data, value, bit_width, bit_offset, is_big_endian);

    // Store as binary string
    let result = Value::from_str(&String::from_utf8_lossy(&data));
    interp.set_var(var_name, result.clone())?;
    Ok(result)
}

/// unpack binValue type bitpos bitwidth
pub fn cmd_unpack(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 5 {
        return Err(Error::wrong_args_with_usage(
            "unpack", 5, args.len(), "binValue type bitpos bitwidth",
        ));
    }
    let bin_data = args[1].as_str().as_bytes();
    let type_name = args[2].as_str();
    let bit_pos = args[3].as_int().ok_or_else(|| {
        Error::runtime(
            format!("expected integer but got \"{}\"", args[3].as_str()),
            ErrorCode::Generic,
        )
    })? as usize;
    let bit_width = args[4].as_int().ok_or_else(|| {
        Error::runtime(
            format!("expected integer but got \"{}\"", args[4].as_str()),
            ErrorCode::Generic,
        )
    })? as usize;

    let is_big_endian = match type_name {
        "be" | "bigendian" => true,
        "le" | "littleendian" | "" => false,
        _ => {
            return Err(Error::runtime(
                format!("unknown type \"{}\": must be le, be, littleendian, or bigendian", type_name),
                ErrorCode::Generic,
            ));
        }
    };

    let value = unpack_int(bin_data, bit_width, bit_pos, is_big_endian);
    Ok(Value::from_int(value))
}

/// Pack an integer value into a byte array at a bit position.
fn pack_int(data: &mut [u8], value: i64, bit_width: usize, bit_offset: usize, big_endian: bool) {
    if bit_width == 0 || bit_width > 64 {
        return;
    }
    let mask = if bit_width == 64 { u64::MAX } else { (1u64 << bit_width) - 1 };
    let val = (value as u64) & mask;

    if bit_offset.is_multiple_of(8) && bit_width.is_multiple_of(8) {
        // Fast path: byte-aligned
        let byte_offset = bit_offset / 8;
        let num_bytes = bit_width / 8;
        let bytes = if big_endian {
            val.to_be_bytes()
        } else {
            val.to_le_bytes()
        };
        if big_endian {
            let start = 8 - num_bytes;
            for i in 0..num_bytes {
                if byte_offset + i < data.len() {
                    data[byte_offset + i] = bytes[start + i];
                }
            }
        } else {
            for i in 0..num_bytes {
                if byte_offset + i < data.len() {
                    data[byte_offset + i] = bytes[i];
                }
            }
        }
    } else {
        // Slow path: bit-by-bit
        for i in 0..bit_width {
            let bit_val = (val >> i) & 1;
            let target_bit = if big_endian {
                bit_offset + bit_width - 1 - i
            } else {
                bit_offset + i
            };
            let byte_idx = target_bit / 8;
            let bit_idx = target_bit % 8;
            if byte_idx < data.len() {
                if bit_val == 1 {
                    data[byte_idx] |= 1 << bit_idx;
                } else {
                    data[byte_idx] &= !(1 << bit_idx);
                }
            }
        }
    }
}

/// Unpack an integer value from a byte array at a bit position.
fn unpack_int(data: &[u8], bit_width: usize, bit_offset: usize, big_endian: bool) -> i64 {
    if bit_width == 0 || bit_width > 64 {
        return 0;
    }

    if bit_offset.is_multiple_of(8) && bit_width.is_multiple_of(8) {
        // Fast path: byte-aligned
        let byte_offset = bit_offset / 8;
        let num_bytes = bit_width / 8;
        let mut bytes = [0u8; 8];
        if big_endian {
            let start = 8 - num_bytes;
            for i in 0..num_bytes {
                if byte_offset + i < data.len() {
                    bytes[start + i] = data[byte_offset + i];
                }
            }
            i64::from_be_bytes(bytes)
        } else {
            for i in 0..num_bytes {
                if byte_offset + i < data.len() {
                    bytes[i] = data[byte_offset + i];
                }
            }
            i64::from_le_bytes(bytes)
        }
    } else {
        // Slow path: bit-by-bit
        let mut val: u64 = 0;
        for i in 0..bit_width {
            let source_bit = if big_endian {
                bit_offset + bit_width - 1 - i
            } else {
                bit_offset + i
            };
            let byte_idx = source_bit / 8;
            let bit_idx = source_bit % 8;
            if byte_idx < data.len() && (data[byte_idx] >> bit_idx) & 1 == 1 {
                val |= 1 << i;
            }
        }
        val as i64
    }
}

// ── Types needed by the interpreter ────────────────────────────────────

/// Alias definition: target command + prefix arguments.
#[derive(Debug, Clone)]
pub struct AliasInfo {
    pub target: String,
    pub prefix_args: Vec<String>,
}

/// Reference info for the ref/getref/setref system.
#[derive(Debug, Clone)]
pub struct RefInfo {
    pub value: Value,
    pub tag: String,
    pub finalizer: Option<String>,
}

/// Saved command for upcall support.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum SavedCommand {
    Proc(super::super::ProcDef),
    Builtin(crate::command::CommandFunc),
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_exists_variable() {
        let mut interp = Interp::new();
        interp.eval("set x 42").unwrap();
        assert_eq!(interp.eval("exists -var x").unwrap().as_str(), "1");
        assert_eq!(interp.eval("exists -var y").unwrap().as_str(), "0");
    }

    #[test]
    fn test_exists_command() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("exists -command set").unwrap().as_str(), "1");
        assert_eq!(interp.eval("exists -command nope").unwrap().as_str(), "0");
    }

    #[test]
    fn test_exists_proc() {
        let mut interp = Interp::new();
        interp.eval("proc foo {} { return 1 }").unwrap();
        assert_eq!(interp.eval("exists -proc foo").unwrap().as_str(), "1");
        assert_eq!(interp.eval("exists -proc bar").unwrap().as_str(), "0");
    }

    #[test]
    fn test_exists_no_qualifier() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("exists set").unwrap().as_str(), "1");
        assert_eq!(interp.eval("exists nope").unwrap().as_str(), "0");
    }

    #[test]
    fn test_alias_basic() {
        let mut interp = Interp::new();
        interp.eval("alias mylist list").unwrap();
        let result = interp.eval("mylist a b c").unwrap();
        assert_eq!(result.as_str(), "a b c");
    }

    #[test]
    fn test_alias_with_prefix() {
        let mut interp = Interp::new();
        interp.eval("alias hello puts -nonewline").unwrap();
        // Just test alias exists
        assert_eq!(interp.eval("exists -alias hello").unwrap().as_str(), "1");
    }

    #[test]
    fn test_ref_getref_setref() {
        let mut interp = Interp::new();
        let handle = interp.eval("ref hello mytag").unwrap();
        let handle_str = handle.as_str().to_string();
        assert!(handle_str.starts_with("<reference.mytag."));

        let val = interp.eval(&format!("getref {}", handle_str)).unwrap();
        assert_eq!(val.as_str(), "hello");

        interp.eval(&format!("setref {} world", handle_str)).unwrap();
        let val2 = interp.eval(&format!("getref {}", handle_str)).unwrap();
        assert_eq!(val2.as_str(), "world");
    }

    #[test]
    fn test_finalize() {
        let mut interp = Interp::new();
        let handle = interp.eval("ref myval tag {puts destroyed}").unwrap();
        let h = handle.as_str().to_string();
        let fin = interp.eval(&format!("finalize {}", h)).unwrap();
        assert_eq!(fin.as_str(), "puts destroyed");
    }

    #[test]
    fn test_stacktrace() {
        let mut interp = Interp::new();
        let result = interp.eval("stacktrace").unwrap();
        // At global level, stack is empty
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_pack_unpack_le() {
        let mut interp = Interp::new();
        interp.eval("pack buf 0x1234 le 16").unwrap();
        let val = interp.eval("unpack [set buf] le 0 16").unwrap();
        assert_eq!(val.as_int(), Some(0x1234));
    }

    #[test]
    fn test_collect() {
        let mut interp = Interp::new();
        let result = interp.eval("collect").unwrap();
        assert_eq!(result.as_str(), "0");
    }
}
