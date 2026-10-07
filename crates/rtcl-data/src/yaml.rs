//! YAML 扩展 — `yaml::decode` 与 `yaml::encode`。
//!
//! 桥接标量约定为本命令自有约定（有意偏离 core json.rs）：bool→"1"/"0"、
//! number 经 serde_yaml 规范化为十进制串（不保留原文写法）；
//! null→`-null` 替换值、string 原样、mapping→dict、sequence→list。
//!
//! ## Commands
//!
//! - `yaml::decode ?-null string? yaml-text`
//! - `yaml::encode value`
//!
//! 亦可作 ensemble：`yaml decode …`、`yaml encode …`（派发按实参位置识别、
//! 与注册名解耦，对 `rename` 免疫）。

use rtcl_core::error::{Error, ErrorCode, Result};
use rtcl_core::interp::Interp;
use rtcl_core::value::{DictMap, Value};

// ── Public entry points ────────────────────────────────────────

/// `yaml` ensemble — dispatches to `decode` / `encode`.
pub fn cmd_yaml(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // ensemble 形态：args[0] = 注册名（`rename` 后可为任意名），args[1] = 子命令词，
    // 载荷/选项自 args[2] 起。派发只认位置（args[1]）、不认注册名字面，
    // 故 `rename yaml y` 后 `y decode …` 依旧正确路由。
    if args.len() < 2 {
        let name = args.first().map(|v| v.as_str()).unwrap_or("yaml");
        return Err(Error::wrong_args(name, 2, args.len()));
    }
    let name = args[0].as_str();
    match args[1].as_str() {
        "decode" => yaml_decode_from(args, 2),
        "encode" => yaml_encode_from(args, 2),
        other => Err(Error::runtime(
            format!("unknown {name} subcommand \"{other}\": must be decode or encode"),
            ErrorCode::InvalidOp,
        )),
    }
}

/// decode 实现 — `start` 由调用方按调用形态显式给定（ensemble = 2，直挂 = 1）。
/// 实现本身绝不嗅探注册名，因此对 `rename` 免疫。
fn yaml_decode_from(args: &[Value], start: usize) -> Result<Value> {
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

/// encode 实现 — 同 [`yaml_decode_from`]，`start` 由调用方显式给定。
fn yaml_encode_from(args: &[Value], start: usize) -> Result<Value> {
    if args.len() <= start {
        return Err(Error::wrong_args_with_usage(
            "yaml::encode", 1, 0, "value"));
    }

    let yaml = value_to_yaml(&args[start]);
    let text = serde_yaml::to_string(&yaml)
        .map_err(|e| Error::runtime(format!("yaml encode: {}", e), ErrorCode::InvalidOp))?;
    Ok(Value::from_str(&text))
}

/// `yaml::decode ?-null string? yaml-text` — 直挂形态：args[0] = 注册名，
/// args[1] 起即选项/载荷（start = 1）。
pub fn cmd_yaml_decode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // 本函数历史上也承接 ensemble 形态实参（args[0] == "yaml" 且带子命令词 →
    // start = 2），该双形态兼容按 args[0] 字面嗅探保留，只服务 Rust 直调方；
    // 经注册名的 ensemble 路径一律走 cmd_yaml 的显式起点，对 rename 免疫。
    // 形态识别必须看 args[0]、绝不能看 args[1]（其值恰可为字面 "decode"）。
    let start = if args.first().is_some_and(|v| v.as_str() == "yaml") { 2 } else { 1 };
    yaml_decode_from(args, start)
}

/// `yaml::encode value` — 直挂形态；双形态兼容同 [`cmd_yaml_decode`]。
pub fn cmd_yaml_encode(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let start = if args.first().is_some_and(|v| v.as_str() == "yaml") { 2 } else { 1 };
    yaml_encode_from(args, start)
}

// ═══════════════════════════════════════════════════════════════
// YAML → VALUE BRIDGE
// ═══════════════════════════════════════════════════════════════

/// Bridge a serde_yaml value to a rtcl Value — 标量表按本命令自有约定：
/// null→`-null` 值、bool→"1"/"0"、number→serde_yaml 规范十进制串、
/// string 原样；mapping→dict，sequence→list。
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

    // ── Task-4 评审遗留：直挂形态 / 顶层标量 / -null 非空替换 ──

    #[test]
    fn direct_mount_forms_skip_no_ensemble_word() {
        // 直挂（宿主直接调 cmd_*，args[0] = 注册名）：start = 1 分支
        let mut i = interp();
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml::decode"), Value::from_str("k: v\n"),
        ]).unwrap();
        assert_eq!(v.as_str(), "k v");

        let e = cmd_yaml_encode(&mut i, &[
            Value::from_str("yaml::encode"), v,
        ]).unwrap();
        assert_eq!(e.as_str(), "k: v\n");

        // 直挂形态同样接受 -null 选项（start=1 之后照常解析）
        let n = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml::decode"), Value::from_str("-null"),
            Value::from_str("~"), Value::from_str("k: null\n"),
        ]).unwrap();
        assert_eq!(n.as_str(), "k ~");
    }

    #[test]
    fn encode_top_level_scalars() {
        // 全部标量经 value_to_yaml 以字符串交给 serde_yaml；
        // 平原串原样输出，会再解析为其他类型的串被单引号钉成字符串
        let mut i = interp();
        let plain = cmd_yaml_encode(&mut i, &[
            Value::from_str("yaml::encode"), Value::from_str("hello"),
        ]).unwrap();
        assert_eq!(plain.as_str(), "hello\n");

        let numeric = cmd_yaml_encode(&mut i, &[
            Value::from_str("yaml::encode"), Value::from_str("3"),
        ]).unwrap();
        assert_eq!(numeric.as_str(), "'3'\n", "钉死 serde_yaml 的实际引号行为");
    }

    // ── Task-6.1 回归：直挂载荷恰为字面 "decode"/"encode" 时不得误判为 ensemble ──

    #[test]
    fn direct_mount_decode_payload_literal_decode() {
        // 直挂形态 args[1] 已是载荷；载荷恰为字面 "decode"（合法 YAML 标量）
        // 曾被 args[1] 哨兵误判成 ensemble 而报 wrong-args
        let mut i = interp();
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml::decode"), Value::from_str("decode"),
        ]).unwrap();
        assert_eq!(v.as_str(), "decode");
    }

    #[test]
    fn direct_mount_encode_payload_literal_encode() {
        let mut i = interp();
        let e = cmd_yaml_encode(&mut i, &[
            Value::from_str("yaml::encode"), Value::from_str("encode"),
        ]).unwrap();
        assert_eq!(e.as_str(), "encode\n");
    }

    #[test]
    fn decode_null_option_non_empty_replacement() {
        let mut i = interp();
        let v = cmd_yaml_decode(&mut i, &[
            Value::from_str("yaml"), Value::from_str("decode"),
            Value::from_str("-null"), Value::from_str("NA"),
            Value::from_str("a: null\nb: 1\n"),
        ]).unwrap();
        assert_eq!(v.as_str(), "a NA b 1");
    }

    // ── 评审回归：ensemble 更名后按新名调用，派发不得依赖注册名字面 ──

    #[test]
    fn ensemble_survives_rename() {
        let mut i = interp();
        crate::register::register(&mut i);
        i.eval("rename yaml y").unwrap();
        // 更名后的 ensemble decode：位置派发，载荷自 args[2] 起
        let v = i.eval("y decode {k: v}").unwrap();
        assert_eq!(v.as_str(), "k v");
        // 更名后的 ensemble encode：dict rep → mapping；标量按本命令约定
        // 以字符串交给 serde_yaml，数字形的 "1" 须被单引号钉成字符串
        let e = i.eval("y encode [dict create a 1]").unwrap();
        assert_eq!(e.as_str(), "a: '1'\n");
        // 直挂不受 ensemble 更名影响：yaml::decode 仍按直挂形态解析标量
        let d = i.eval("yaml::decode decode").unwrap();
        assert_eq!(d.as_str(), "decode");
    }
}
