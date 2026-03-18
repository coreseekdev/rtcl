//! Package system: package provide/require/names/forget/vcompare/vsatisfies.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

pub fn cmd_package(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "package", 2, args.len(),
            "package subcommand ?arg ...?",
        ));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        "provide" => pkg_provide(interp, args),
        "require" => pkg_require(interp, args),
        "names"   => pkg_names(interp),
        "forget"  => pkg_forget(interp, args),
        "vcompare" => pkg_vcompare(args),
        "vsatisfies" => pkg_vsatisfies(args),
        _ => Err(Error::runtime(
            format!("unknown package subcommand \"{}\": must be forget, names, provide, require, vcompare, or vsatisfies", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

/// `package provide name ?version?`
fn pkg_provide(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "package provide", 3, args.len(),
            "package provide name ?version?",
        ));
    }
    let name = args[2].as_str();
    if args.len() == 4 {
        let version = args[3].as_str().to_string();
        interp.packages.insert(name.to_string(), version);
        Ok(Value::empty())
    } else {
        // Query: return the version if provided
        match interp.packages.get(name) {
            Some(v) => Ok(Value::from_str(v)),
            None => Ok(Value::empty()),
        }
    }
}

/// `package require ?-exact? name ?version?`
fn pkg_require(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "package require", 3, args.len(),
            "package require ?-exact? name ?version?",
        ));
    }

    let mut i = 2;
    let _exact = if args[i].as_str() == "-exact" {
        i += 1;
        true
    } else {
        false
    };

    if i >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "package require", 3, args.len(),
            "package require ?-exact? name ?version?",
        ));
    }

    let name = args[i].as_str();

    // Check if already provided
    if let Some(v) = interp.packages.get(name) {
        return Ok(Value::from_str(v));
    }

    // Try to auto-load from $auto_path
    #[cfg(feature = "file")]
    {
        let auto_path = interp.globals.get("auto_path")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        if !auto_path.is_empty() {
            if let Some(list) = Value::from_str(&auto_path).as_list() {
                for dir in &list {
                    let pkgindex = format!("{}/pkgIndex.tcl", dir.as_str());
                    if std::path::Path::new(&pkgindex).exists() {
                        // Source the pkgIndex.tcl (it may call `package provide`)
                        let _ = interp.eval(&format!("source {{{}}}", pkgindex));
                        // Check if now available
                        if let Some(v) = interp.packages.get(name) {
                            return Ok(Value::from_str(v));
                        }
                    }
                }
            }
        }
    }

    Err(Error::runtime(
        format!("can't find package {}", name),
        crate::error::ErrorCode::NotFound,
    ))
}

/// `package names`
fn pkg_names(interp: &mut Interp) -> Result<Value> {
    let mut names: Vec<Value> = interp.packages.keys()
        .map(|k| Value::from_str(k))
        .collect();
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(Value::from_list(&names))
}

/// `package forget ?name ...?`
fn pkg_forget(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    for arg in &args[2..] {
        interp.packages.remove(arg.as_str());
    }
    Ok(Value::empty())
}

/// `package vcompare version1 version2`
fn pkg_vcompare(args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        return Err(Error::wrong_args_with_usage(
            "package vcompare", 4, args.len(),
            "package vcompare version1 version2",
        ));
    }
    let cmp = compare_versions(args[2].as_str(), args[3].as_str());
    Ok(Value::from_int(cmp as i64))
}

/// `package vsatisfies version requirement`
fn pkg_vsatisfies(args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        return Err(Error::wrong_args_with_usage(
            "package vsatisfies", 4, args.len(),
            "package vsatisfies version requirement",
        ));
    }
    let cmp = compare_versions(args[2].as_str(), args[3].as_str());
    Ok(Value::from_bool(cmp >= 0))
}

/// Compare two version strings component-by-component.
/// Returns -1, 0, or 1.
fn compare_versions(a: &str, b: &str) -> i32 {
    let pa: Vec<i64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let pb: Vec<i64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        if va < vb { return -1; }
        if va > vb { return 1; }
    }
    0
}
