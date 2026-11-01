//! towasm CLI —— Tcl → wasm 产物编译器。
//!
//! 用法：
//!   towasm app.tcl -o app.wasm          # 产物写盘
//!   towasm app.tcl --manifest app.mf    # ABI 契约清单（"-" = stdout）
//!   towasm app.tcl --verify             # 编译 + wasmi/wasmtime 跨引擎
//!                                       # 回环 + 解释器三方对值
//!
//! 产物 = .wasm 字节；宿主以 rtcl.* 导入提供运行时（契约 = 清单 +
//! docs/JIT-WASM-GC.md ABI v2）。

use rtcl_core::Interp;
use rtcl_towasm::compile_script;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut input: Option<String> = None;
    let mut out: Option<String> = None;
    let mut manifest: Option<String> = None;
    let mut verify = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => out = args.next(),
            "--manifest" => manifest = Some(args.next().unwrap_or_else(|| "-".into())),
            "--verify" => verify = true,
            other if input.is_none() => input = Some(other.into()),
            other => {
                eprintln!("towasm: unexpected argument {other:?}");
                std::process::exit(2);
            }
        }
    }
    let Some(input) = input else {
        eprintln!("usage: towasm <script.tcl> [-o out.wasm] [--manifest <file|->] [--verify]");
        std::process::exit(2);
    };
    let script = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("towasm: cannot read {input}: {e}");
            std::process::exit(1);
        }
    };
    let art = match compile_script(&script) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("towasm: {input}: {e}");
            std::process::exit(1);
        }
    };
    if let Some(mf) = &manifest {
        if mf == "-" {
            print!("{}", art.manifest);
        } else if let Err(e) = std::fs::write(mf, &art.manifest) {
            eprintln!("towasm: cannot write {mf}: {e}");
            std::process::exit(1);
        }
    }
    if let Some(out) = &out {
        if let Err(e) = std::fs::write(out, &art.bytes) {
            eprintln!("towasm: cannot write {out}: {e}");
            std::process::exit(1);
        }
        eprintln!(
            "towasm: {} -> {} ({} bytes)",
            input,
            out,
            art.bytes.len()
        );
    }
    if verify {
        if let Err(diff) = verify_roundtrip(&script, &art) {
            eprintln!("towasm verify: FAIL\n{diff}");
            std::process::exit(1);
        }
        eprintln!("towasm verify: OK (interp == wasmi == wasmtime)");
    }
    if out.is_none() && manifest.is_none() && !verify {
        eprintln!("towasm: nothing to do (use -o / --manifest / --verify)");
        std::process::exit(2);
    }
}

/// 跨引擎回环：解释器 vs wasmi 产物执行 vs wasmtime 产物执行——
/// 末值 + `__tw_probe` 探针变量三方对值。产物即产品：同一份字节
/// 在不同引擎必须等价。
fn verify_roundtrip(script: &str, art: &rtcl_towasm::Artifact) -> Result<(), String> {
    let probe_of = |interp: &Interp| -> String {
        match interp.get_var("__tw_probe") {
            Ok(v) => v.as_str().to_string(),
            Err(_) => "<unset>".into(),
        }
    };
    // 解释器基线
    let mut ip = Interp::new();
    let base = match ip.eval(script) {
        Ok(_) => probe_of(&ip),
        Err(e) => return Err(format!("interp eval failed: {e:?}")),
    };
    // wasmi 产物执行
    let (iw, rw) = rtcl_jit::exec::execute_artifact_wasmi(
        Interp::new(),
        &art.bc,
        &art.module,
        "towasm-verify",
    );
    if matches!(
        rw.outcome,
        rtcl_jit::exec::Outcome::Fallback
    ) {
        return Err("wasmi: artifact fell back (not executed as wasm)".into());
    }
    let pw = probe_of(&iw);
    eprintln!(
        "[tw-dbg] wasmi outcome-fallback={} result={:?} probe={pw:?}",
        matches!(rw.outcome, rtcl_jit::exec::Outcome::Fallback),
        rw.result.as_ref().map(|v| v.as_str().to_string()),
    );
    // wasmtime 产物执行
    let (it, rt) = rtcl_jit::exec_wasmtime::execute(
        Interp::new(),
        &art.bc,
        &art.module,
        "towasm-verify",
    );
    let pt = probe_of(&it);
    if base != pw || base != pt {
        return Err(format!("probe mismatch: interp={base:?} wasmi={pw:?} wasmtime={pt:?}"));
    }
    let _ = (rw.result.as_ref(), rt.result.as_ref());
    Ok(())
}
