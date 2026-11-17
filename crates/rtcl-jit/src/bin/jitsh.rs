//! jitsh — the JIT-engine CLI（judge `--engine jit` 档的执行体）。
//!
//! 文件模式逐字节复刻 rtcl-cli `run_file` 的行为（正常路径无输出差异、
//! 错误路径 `error_report` 逐字节一致、`exit N`/`return N` 的顶层处置
//! 相同）；顶层单元被 JIT 接受时走 wasm 执行，否则回退 `interp.eval`
//! （构造性等价）。

use rtcl_core::Interp;
use rtcl_jit::exec::{self, Outcome};

fn main() {
    // 启动一次性 CLI 诊断（fuzz 接受性探针；绝不进每 op 路径）：
    // RTCL_JIT_ACCEPT=1 时对脚本做接受性判定后退出。
    if std::env::var_os("RTCL_JIT_ACCEPT").is_some() {
        accept_probe();
        return;
    }
    // M2 seam：注册 proc-body JIT 钩子（jitsh 是 JIT 引擎执行体）。
    // M4「定义即编译」默认档（env RTCL_JIT_AUTOCOMPILE=0 可关——逃生口
    // 优先于本默认；judge/battery/fuzz 全门禁经 jitsh 走此路径）。
    rtcl_jit::proc_exec::set_autocompile(true);
    rtcl_jit::proc_exec::install();
    let mut args = std::env::args().skip(1);
    let file = match (args.next().as_deref(), args.next()) {
        (Some("-f"), Some(f)) => f,
        (Some("-c"), Some(c)) => {
            run_command(&c);
            return;
        }
        _ => {
            eprintln!("usage: jitsh -f <file> | -c <script>");
            std::process::exit(2);
        }
    };
    let mut interp = Interp::new();
    interp.set_script_name(&file);
    // 文件执行语境（rtcl-cli run_file 同款）：errorInfo 的文件级
    // if/while/for 体帧归属以此判别（unwind_subs 的 in_file_script 臂
    // ——proc 体在 call_proc 边界强制编译语境，不受影响）。arm 经
    // eval_impl 消费；JIT 顶层单元不经 eval_impl，故直设同状态。
    interp.arm_file_script();
    interp.jit_err_set_file_script(true);
    let script = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: cannot read file '{}': {}", file, e);
            rtcl_jit::proc_exec::dump_autocompile_stats();
            std::process::exit(1);
        }
    };
    let (mut interp, run) = exec::run_top(interp, &script, &file);
    match run.outcome {
        Outcome::Fallback => {
            if let Err(e) = interp.eval(&script) {
                // 顶层控制流解码（D-11，rtcl-cli run_file 同款）：
                // `return -code error/break/continue` 与裸 break/
                // continue 在顶层就是错误（完整 errorInfo rc 1），普通
                // return 正常完成 rc 0 静默，`exit N` 以 N 退出。
                let e = interp.decode_toplevel_flow(e);
                if e.is_exit() {
                    let code = e.message_text().trim().parse::<i32>().unwrap_or(0);
                    rtcl_jit::proc_exec::dump_autocompile_stats();
                    std::process::exit(code);
                }
                if e.is_return() {
                    return;
                }
                let report = interp.error_report(&e, &file);
                eprintln!("{}", report);
                rtcl_jit::proc_exec::dump_autocompile_stats();
                std::process::exit(1);
            }
        }
        Outcome::Completed { code, exit_code } => match code {
            // 5 = STALE：epoch 回边比对失败（M4 模块缓存时代的信号）。
            // 当前驱动语义是仅单元入口 gate（run_top_with），运行中
            // epoch global 不刷新 → 不可能产生 STALE；维持忽略仅为
            // 完整覆盖返回码空间。
            0 | 5 => {}
            // 顶层 return（含带值）= 正常完成（tclsh 文件模式：rc 0、
            // 静默——与 rtcl-cli run_file 2026-10-19 对齐同步）。
            2 => {}
            6 => {
                // exit N：以 N 退出且无输出（tclsh 对齐）。
                rtcl_jit::proc_exec::dump_autocompile_stats();
                std::process::exit(exit_code.unwrap_or(0));
            }
            _ => {
                if let Some(e) = run.eval_error {
                    let report = interp.error_report(&e, &file);
                    eprintln!("{}", report);
                } else if let Some(r) = run.driver_report {
                    if !r.is_empty() {
                        eprintln!("{}", r);
                    }
                }
                rtcl_jit::proc_exec::dump_autocompile_stats();
                std::process::exit(1);
            }
        },
    }
}

fn run_command(cmd: &str) {
    // -c 语境 = rtcl-cli run_command 同款：eval_incremental 增量求值
    //（stdin 语义，非文件模式的整脚本编译执行），顶层处置相同——
    // `exit N` 以 N 退出、return（含 deferred）正常完成 rc 0、break/
    // continue 原文 `Error: <text>` rc 1、错误 `Error: <msg>`（无帧，
    // 文件级帧归属只属 -f）。JIT 差分门禁全走 -f（judge/battery/
    // probes/fuzz 的比较路径），-c 保留纯解释器探针面 + RTCL_JIT_
    // ACCEPT 接受性判定。
    let mut interp = Interp::new();
    let result = match interp.eval_incremental(cmd) {
        Ok(r) => r,
        Err(e) if e.is_exit() => {
            let code = e.message_text().trim().parse::<i32>().unwrap_or(0);
            std::process::exit(code);
        }
        Err(e) if e.is_return() => return,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    if !result.is_empty() {
        println!("{}", result);
    }
}

/// 接受性探针：parse → compile_unit → unfused → emit::compile。
fn accept_probe() {
    use rtcl_parser::{Compiler, ScriptUnit};
    use std::rc::Rc;
    // argv[1] = "-c"/"-f"（accept_probe 在 argv[0]=jitsh 后直接取 1/2）。
    let script = match (std::env::args().nth(1).as_deref(), std::env::args().nth(2)) {
        (Some("-c"), Some(c)) => c,
        (Some("-f"), Some(f)) => match std::fs::read_to_string(&f) {
            Ok(s) => s,
            Err(e) => {
                println!("REJECT: io {e}");
                return;
            }
        },
        _ => {
            println!("REJECT: no input");
            return;
        }
    };
    let verdict = match ScriptUnit::parse(&script) {
        Ok(unit) => {
            let bc = Compiler::compile_unit(Rc::clone(&unit.source), &unit.commands).unfused();
            match rtcl_jit::emit::compile(&bc, &Default::default()) {
                Ok(_) => "ACCEPT".to_string(),
                Err(e) => format!("REJECT: {e}"),
            }
        }
        Err(e) => format!("REJECT: parse {e}"),
    };
    println!("{verdict}");
}
