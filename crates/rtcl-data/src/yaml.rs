//! YAML extension — `yaml::decode` and `yaml::encode` (mirrors core json.rs
//! bridging conventions: `-null` option, bool→"1"/"0", numbers→canonical
//! decimal strings, mapping→dict, sequence→list).
//!
//! ## Commands
//!
//! - `yaml::decode ?-null string? yaml-text`
//! - `yaml::encode value`
//!
//! Also available as ensemble: `yaml decode ...`, `yaml encode ...`.

use rtcl_core::error::{Error, ErrorCode, Result};
use rtcl_core::interp::Interp;
use rtcl_core::value::{DictMap, Value};

// ── Public entry points ────────────────────────────────────────

/// `yaml` ensemble — dispatches to `decode` / `encode`.
pub fn cmd_yaml(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("yaml", 2, args.len()));
    }
    match args[1].as_str() {
        "decode" => cmd_yaml_decode(interp, args),
        "encode" => cmd_yaml_encode(interp, args),
        other => Err(Error::runtime(
            format!("unknown yaml subcommand \"{}\": must be decode or encode", other),
            ErrorCode::InvalidOp,
        )),
    }
}

/// `yaml::decode ?-null string? yaml-text`
pub fn cmd_yaml_decode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Parse options — skip args[0] ("yaml") and args[1] ("decode")
    let start = if args.len() > 1 && args[1].as_str() == "decode" { 2 } else { 1 };
    let mut null_value = "null".to_string();
    let mut i = start;

    while i < args.len() {
        match args[i].as_str() {
            "-null" => {
                i += 1;
                if i >= args.len() {
                    return Err(Error::runtime(
                        "-null requires a value", ErrorCode::InvalidOp));
                }
                null_value = args[i].as_str().to_string();
                i += 1;
            }
            _ => break,
        }
    }
    if i >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "yaml::decode", 1, 0, "?-null string? yaml-text"));
    }
    let yaml_str = args[i].as_str();

    if yaml_str.is_empty() {
        return Err(Error::runtime("empty YAML string", ErrorCode::InvalidOp));
    }

    let doc: serde_yaml::Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| Error::runtime(format!("yaml parse: {}", e), ErrorCode::InvalidOp))?;

    Ok(bridge(&doc, &null_value))
}

/// `yaml::encode value`
pub fn cmd_yaml_encode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let start = if args.len() > 1 && args[1].as_str() == "encode" { 2 } else { 1 };
    if args.len() <= start {
        return Err(Error::wrong_args_with_usage(
            "yaml::encode", 1, 0, "value"));
    }

    let yaml = value_to_yaml(&args[start]);
    let text = serde_yaml::to_string(&yaml)
        .map_err(|e| Error::runtime(format!("yaml encode: {}", e), ErrorCode::InvalidOp))?;
    Ok(Value::from_str(&text))
}

// ═══════════════════════════════════════════════════════════════
// YAML → VALUE BRIDGE
// ═══════════════════════════════════════════════════════════════

/// Bridge a serde_yaml value to a rtcl Value (scalar table mirrors json.rs
/// decode: null→`-null` value, bool→"1"/"0", number→canonical decimal string,
/// string→verbatim; mapping→dict, sequence→list).
fn bridge(sv: &serde_yaml::Value, null_repl: &str) -> Value {
    match sv {
        serde_yaml::Value::Null => Value::from_str(null_repl),
        serde_yaml::Value::Bool(b) => Value::from_str(if *b { "1" } else { "0" }),
        serde_yaml::Value::Number(n) => Value::from_str(&n.to_string()),
        serde_yaml::Value::String(s) => Value::from_str(s),
        serde_yaml::Value::Sequence(items) => {
            Value::from_list_cached(items.iter().map(|it| bridge(it, null_repl)).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut entries = DictMap::ordered_with_capacity(map.len());
            for (k, v) in map {
                let key = bridge(k, null_repl).as_str().to_string();
                entries.insert(key, bridge(v, null_repl));
            }
            Value::from_dict_cached(entries)
        }
        // Tagged scalars (`!!str foo`): the tag is dropped, the inner value bridges.
        serde_yaml::Value::Tagged(tagged) => bridge(&tagged.value, null_repl),
    }
}

// ═══════════════════════════════════════════════════════════════
// VALUE → YAML BRIDGE
// ═══════════════════════════════════════════════════════════════

/// Bridge a rtcl Value to a serde_yaml value — the symmetric inverse of
/// [`bridge`]: dict rep→mapping, list rep→sequence, anything else→string.
/// Dispatch is internal-rep based (like json.rs's zero-copy `as_dict` fast
/// path); scalars are handed to serde_yaml as strings, which quotes them
/// when the plain form would re-parse as a different type.
fn value_to_yaml(value: &Value) -> serde_yaml::Value {
    if let Some(map) = value.as_dict_ref() {
        let mut out = serde_yaml::Mapping::with_capacity(map.len());
        for (k, v) in map {
            out.insert(
                serde_yaml::Value::String(k.clone()),
                value_to_yaml(v),
            );
        }
        serde_yaml::Value::Mapping(out)
    } else if let Some(items) = value.as_list_ref() {
        serde_yaml::Value::Sequence(items.iter().map(value_to_yaml).collect())
    } else {
        serde_yaml::Value::String(value.as_str().to_string())
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use rtcl_core::interp::Interp;

    fn interp() -> Interp {
        Interp::new()
    }

    #[test]
    fn decode_flat_dict_exact() {
        let mut i = interp();
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"),
            Value::from_str("name: SAP\ncount: 3\nactive: true\n"),
        ]).unwrap();
        // 平面 dict 的规范 Tcl 串 = "k v k v …"；bool→1（标量映射表约定）
        assert_eq!(v.as_str(), "name SAP count 3 active 1");
    }

    #[test]
    fn decode_null_option() {
        let mut i = interp();
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"),
            Value::from_str("-null"), Value::from_str(""),
            Value::from_str("x: null\n"),
        ]).unwrap();
        assert_eq!(v.as_str(), "x {}"); // 空串在 dict 值位以 {} 形式呈现
    }

    #[test]
    fn encode_decode_roundtrip_nested() {
        let mut i = interp();
        let src = "a:\n  - 1\n  - two\nb: {c: d}\n";
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"), Value::from_str(src),
        ]).unwrap();
        let enc = cmd_yaml_encode(&mut i, &[Value::from_str("yaml"), Value::from_str("encode"), v]).unwrap();
        let v2 = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"), enc,
        ]).unwrap();
        // 结构不变式：decode∘encode∘decode == decode（串形以 rtcl 规范形式为准）
        let v1 = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"), Value::from_str(src),
        ]).unwrap();
        let d1 = cmd_yaml_encode(&mut i, &[Value::from_str("yaml"), Value::from_str("encode"), v1]).unwrap();
        let d2 = cmd_yaml_encode(&mut i, &[Value::from_str("yaml"), Value::from_str("encode"), v2]).unwrap();
        assert_eq!(d1.as_str(), d2.as_str());
    }

    #[test]
    fn decode_bad_yaml_is_runtime_error() {
        let mut i = interp();
        let r = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"),
            Value::from_str("a: [unclosed"),
        ]);
        assert!(r.is_err());
    }

    #[test]
    fn ensemble_dispatch() {
        let mut i = interp();
        let v = cmd_yaml(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"), Value::from_str("k: v\n"),
        ]).unwrap();
        assert_eq!(v.as_str(), "k v");
        let e = cmd_yaml(&mut i, &[
            Value::from_str("yaml"), Value::from_str("encode"), v.clone(),
        ]).unwrap();
        assert_eq!(e.as_str(), "k: v\n");
        let again = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"), e,
        ]).unwrap();
        assert_eq!(again.as_str(), v.as_str());
    }

    #[test]
    fn unknown_subcommand_is_error() {
        let mut i = interp();
        let r = cmd_yaml(&mut i, &[
            Value::from_str("yaml"), Value::from_str("frobnicate"),
        ]);
        assert!(r.is_err());
    }
}
