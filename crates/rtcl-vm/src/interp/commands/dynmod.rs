//! Dynamic module registry — the `load`/`unload` surface for
//! host-instantiated extension modules (the wasm story; the native
//! dlopen form stays behind `load-cmd`).
//!
//! rtcl's `load` differs from tclsh's by necessity: a wasm side module
//! cannot reach into the interpreter and self-register commands (no
//! shared address space).  Instead the HOST instantiates the module
//! ahead of time and *declares* it via [`Interp::register_dyn_module`],
//! passing, per command, the dispatch words that invoke it (a bridge
//! command plus its fixed leading arguments — rtcl-wasm wires these to
//! the module's exports through the `_js_dispatch` table).  `load name`
//! then installs the commands synchronously; `unload name` removes them.
//!
//! Dispatch words: for `commands = [("add", ["_js_dispatch 3
//! __dynmod:myext:add"])]`, `load myext` generates
//! `proc ::myext::add {args} { _js_dispatch 3 __dynmod:myext:add {*}$args }`
//! — the same proc-wrapper bridge shape the JS command surface uses.
//!
//! Path-form loads (`load /x/y.so`) keep tclsh's dlopen semantics via
//! the `load-cmd` fallback; an unknown file yields tclsh's exact error
//! text on every platform.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

/// 调用参数传递协议（ABI 版本）。
///
/// - [`DynAbi::V1Join`]：wrapper 体 `[join $args "\u0001"]`——分隔符
///   单词。数据含 \u0001 时会在宿主拆分处丢界（v1 限制）。
/// - [`DynAbi::V2List`]（推荐）：wrapper 体 `<words> [llength $args]
///   $args`——参数以**列表值单词**（rtcl 序列化，round-trip 安全）+
///   前置词数传递，任意 Unicode / \u0001 数据保真。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DynAbi {
    #[default]
    V1Join,
    V2List,
}

#[derive(Clone, Debug)]
pub struct DynModuleDef {
    /// `(command tail, dispatch words)` — the wrapper body calls the
    /// bridge as `<words...> <args-form>`（按 [`DynAbi`] 取形）。
    pub commands: Vec<(String, Vec<String>)>,
    /// 调用协议版本（v2 = [`DynAbi::V2List`]）。
    pub abi: DynAbi,
}

/// Load state of a registered module.
#[derive(Clone, Debug)]
pub struct DynModule {
    pub def: DynModuleDef,
    /// Command full names installed by `load` (removed by `unload`).
    pub installed: Vec<String>,
}

impl Interp {
    /// Declare a host-instantiated extension module.  Idempotent: a
    /// re-declaration replaces the previous definition (tclsh's `load`
    /// of an already-loaded file is equally forgiving).
    pub fn register_dyn_module(&mut self, name: &str, def: DynModuleDef) {
        let key = name.trim_start_matches("::").to_string();
        self.dyn_modules.insert(key, DynModule { def, installed: Vec::new() });
    }

    /// v2 形态便捷注册：abi = [`DynAbi::V2List`]（列表值单词协议）。
    pub fn register_dyn_module_v2(&mut self, name: &str, mut def: DynModuleDef) {
        def.abi = DynAbi::V2List;
        self.register_dyn_module(name, def);
    }

    /// Commands a loaded module installed (for host-side cleanup).
    pub fn dyn_module_installed_commands(&self, name: &str) -> Vec<String> {
        self.dyn_modules
            .get(name.trim_start_matches("::"))
            .map(|rec| rec.installed.clone())
            .unwrap_or_default()
    }

    /// Drop a module declaration (host dropped the instance).
    pub fn unregister_dyn_module(&mut self, name: &str) -> bool {
        let key = name.trim_start_matches("::");
        let removed = self.dyn_modules.remove(key);
        if let Some(rec) = &removed {
            if !rec.installed.is_empty() {
                // Also uninstall the commands (the instance is gone).
                let installed = rec.installed.clone();
                for full in installed {
                    let _ = self.eval(&format!("rename {} \"\"", crate::value::tcl_quote(&full)));
                }
            }
        }
        removed.is_some()
    }

    /// Registry lookup: exact name, then the stem of a file-style
    /// argument (`/path/to/myext.so` → `myext`).
    fn dyn_module_lookup(&self, arg: &str) -> Option<String> {
        let bare = arg.trim_start_matches("::");
        if self.dyn_modules.contains_key(bare) {
            return Some(bare.to_string());
        }
        let stem = arg.rsplit('/').next().unwrap_or(arg);
        let stem = stem
            .strip_suffix(".so")
            .or_else(|| stem.strip_suffix(".dll"))
            .or_else(|| stem.strip_suffix(".wasm"))
            .or_else(|| stem.strip_suffix(".dylib"))
            .unwrap_or(stem);
        if self.dyn_modules.contains_key(stem) {
            return Some(stem.to_string());
        }
        None
    }
}

/// `load ?-global? ?-lazy? ?--? fileName ?packageName? ?interp?` —
/// registry form first, tclsh's dlopen form as the fallback.
pub fn cmd_load(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Option window mirrors tclsh's load: the flags precede the file.
    let mut i = 1;
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-global" | "-lazy" | "--" => i += 1,
            _ => break,
        }
    }
    if args.len() - i < 1 {
        return Err(Error::wrong_args_with_usage(
            "load",
            1,
            args.len(),
            "?-global? ?-lazy? ?--? fileName ?packageName? ?interp?",
        ));
    }
    let file = args[i].as_str();

    if let Some(key) = interp.dyn_module_lookup(file) {
        let already_loaded = interp
            .dyn_modules
            .get(&key)
            .map(|rec| !rec.installed.is_empty())
            .unwrap_or(false);
        if already_loaded {
            // Already loaded: tclsh re-load of a loaded file is a no-op.
            return Ok(Value::empty());
        }
        let (snapshot, abi) = {
            let rec = interp.dyn_modules.get(&key).expect("record");
            (rec.def.commands.clone(), rec.def.abi)
        };
        let mut installed = Vec::new();
        for (cmd, words) in &snapshot {
            let full = format!("::{}::{}", key, cmd);
            let mut call = String::new();
            for w in words {
                call.push_str(&crate::value::tcl_quote(w));
                call.push(' ');
            }
            match abi {
                // v2：`{*}$args` 直传——expand 元素感知（expand.probes
                // 11/11 已证整词保真），无分隔符歧义；宿主 bridge 逐词
                // 收参（\u0001 数据安全，v1 join 的 \u0001 丢界解除）。
                DynAbi::V2List => call.push_str("{*}$args"),
                DynAbi::V1Join => call.push_str("[join $args \"\\u0001\"]"),
            }
            let script = format!(
                "proc {} {{args}} {{ {} }}",
                crate::value::tcl_quote(&full),
                call.trim_end()
            );
            interp.eval(&script)?;
            installed.push(full);
        }
        interp.ensure_ns_path(&format!("::{}", key));
        let rec = interp.dyn_modules.get_mut(&key).expect("record");
        rec.installed = installed;
        return Ok(Value::empty());
    }

    #[cfg(feature = "load-cmd")]
    {
        return super::compat::cmd_load_dlopen(interp, args);
    }
    #[cfg(not(feature = "load-cmd"))]
    {
        let _ = file;
        Err(Error::runtime(
            "dynamic loading is not supported in this rtcl build",
            ErrorCode::Io,
        ))
    }
}

/// `unload ?-switch ...? fileName ?packageName? ?interp?` — registry
/// form first, dlopen fallback.
pub fn cmd_unload(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "unload",
            1,
            args.len(),
            "?-switch ...? fileName ?packageName? ?interp?",
        ));
    }
    let file = args[1].as_str();
    if let Some(key) = interp.dyn_module_lookup(file) {
        let rec = interp.dyn_modules.get(&key).expect("record");
        if rec.installed.is_empty() {
            // tclsh: unload of something never loaded.
            return Err(Error::runtime(
                format!("file \"{}\" has never been loaded", file),
                ErrorCode::Generic,
            ));
        }
        let installed = rec.installed.clone();
        for full in installed {
            let _ = interp.eval(&format!("rename {} \"\"", crate::value::tcl_quote(&full)));
        }
        let rec = interp.dyn_modules.get_mut(&key).expect("record");
        rec.installed.clear();
        return Ok(Value::empty());
    }

    #[cfg(feature = "load-cmd")]
    {
        return super::compat::cmd_unload_dlopen(interp, args);
    }
    #[cfg(not(feature = "load-cmd"))]
    {
        Err(Error::runtime(
            format!("file \"{}\" has never been loaded", file),
            ErrorCode::Generic,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use crate::interp::Interp;
    use crate::interp::commands::dynmod::DynAbi;
    use crate::value::Value;
    use std::cell::RefCell;

    thread_local! {
        static BRIDGE_LOG: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
    }

    fn ev(script: &str) -> String {
        let mut interp = Interp::new();
        // join 协议（wrapper 体 `[join $args "\u0001"]`）：dispatch
        // words 是独立词，**最后一个词**是 \u0001 连接的调用参数串——
        // native 宿主的 bridge 自行拆分（wasm 侧 js_dispatch 统一拆）。
        interp.register_command("fake_bridge", |_interp, args| {
            let words: Vec<String> = args.iter().map(|a| a.as_str().to_string()).collect();
            BRIDGE_LOG.with(|log| log.borrow_mut().push(words.clone()));
            // words = [fake_bridge, myext, add, "20\u{1}22"]
            let params: Vec<&str> = words
                .last()
                .map(|w| w.split('\u{0001}').collect())
                .unwrap_or_default();
            if words.len() == 4 && words[2] == "add" && params.len() == 2 {
                let a: i64 = params[0].parse().unwrap_or(0);
                let b: i64 = params[1].parse().unwrap_or(0);
                return Ok(Value::from_int(a + b));
            }
            Ok(Value::from_str("bridged"))
        });
        interp.register_dyn_module(
            "myext",
            super::DynModuleDef {
                commands: vec![
                    ("add".to_string(),
                     vec!["fake_bridge".to_string(), "myext".to_string(), "add".to_string()]),
                    ("version".to_string(),
                     vec!["fake_bridge".to_string(), "myext".to_string(), "version".to_string()]),
                ],
                abi: super::DynAbi::V1Join,
            },
        );
        interp.eval(script).unwrap().as_str().to_string()
    }

    #[test]
    fn load_installs_and_dispatches() {
        let out = ev("load myext; myext::add 20 22");
        assert_eq!(out, "42");
    }

    #[test]
    fn load_is_listed_and_idempotent() {
        let out = ev("set r {}; load myext; lappend r [info commands myext::*]; load myext; lappend r [myext::version]; join $r ,");
        assert!(out.contains("::myext::add"), "listing: {out}");
        assert!(out.contains("bridged"), "re-load no-op + call: {out}");
    }

    #[test]
    fn unload_removes_and_reports() {
        let out = ev("load myext; unload myext; catch {myext::add 1 1} e; list [info commands myext::*] [string match {*invalid command name*} $e]");
        assert_eq!(out, "{} 1");
        let err = Interp::new_err("unload myext");
        assert_eq!(err, "file \"myext\" has never been loaded");
    }

    #[test]
    fn unknown_path_falls_back_to_faithful_error() {
        let err = Interp::new_err("load /nonexistent.so");
        assert_eq!(
            err,
            "couldn't load file \"/nonexistent.so\": /nonexistent.so: cannot open shared object file: No such file or directory"
        );
    }

    #[test]
    fn stem_lookup_via_path_form() {
        let out = ev("load /somewhere/myext.wasm; myext::add 1 2");
        assert_eq!(out, "3");
    }

    /// v2（列表值单词协议）数据保真：\u0001 / 空白 / 全 Unicode 词
    /// 逐词到达宿主 bridge（v1 join 协议对 \u0001 数据丢界）。
    #[test]
    fn v2_abi_binary_and_unicode_safe() {
        use std::cell::RefCell;
        thread_local! {
            static RX: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
        }
        let mut interp = Interp::new();
        // v2 wrapper 逐词直传：bridge 收到的词（跳过前 3 个 dispatch
        // 词）= 调用参数原词（\u0001 / 空白 / 全 Unicode 保真）。
        interp.register_command("v2_bridge", |_interp, args| {
            let words: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            RX.with(|log| log.borrow_mut().push(words[3..].to_vec()));
            Ok(Value::from_str("v2ok"))
        });
        interp.register_dyn_module_v2(
            "binmod",
            super::DynModuleDef {
                commands: vec![("echo".to_string(),
                    vec!["v2_bridge".to_string(), "binmod".to_string(), "echo".to_string()])],
                abi: DynAbi::V2List,
            },
        );
        interp.eval("load binmod").unwrap();
        // load 装 ::模块::命令（用户面入口是带命名空间的全名）；
        // 逐条调用（数据含 \u0001 字面 / 空白 / 全 Unicode——v2 列表值
        // 单词协议保真；v1 join 协议对 \u0001 数据丢界）
        let cases: Vec<Vec<String>> = vec![
            vec!["a\u{1}b".to_string(), "普通文本".to_string()],
            vec!["日语🈁".to_string(), "x y z".to_string(), "".to_string()],
        ];
        for case in &cases {
            let quoted: Vec<String> = case.iter().map(|c| crate::value::tcl_quote(c)).collect();
            let script = format!("::binmod::echo {}", quoted.join(" "));
            let out = interp.eval(&script).unwrap();
            assert_eq!(out.as_str(), "v2ok");
            let last = RX.with(|l| l.borrow().last().cloned().unwrap_or_default());
            assert_eq!(&last, case, "v2 逐词保真: {last:?} vs {case:?}");
        }
    }

    impl Interp {
        fn new_err(script: &str) -> String {
            Interp::new().eval(script).unwrap_err().to_string()
        }
    }
}
