//! jitsh — the JIT-engine CLI（judge `--engine jit` 档的执行体）。
//!
//! 文件模式逐字节复刻 rtcl-cli `run_file` 的行为（正常路径无输出差异、
//! 错误路径 `error_report` 逐字节一致、`exit N`/`return N` 的顶层处置
//! 相同）；顶层单元被 JIT 接受时走 wasm 执行，否则回退 `interp.eval`
//! （构造性等价）。

use rtcl_core::Interp;
use rtcl_jit::exec::{self, Outcome};

fn main() {
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
    let script = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read file '{}': {}", file, e);
            std::process::exit(1);
        }
    };
    let (mut interp, run) = exec::run_top(interp, &script, &file);
    match run.outcome {
        Outcome::Fallback => {
            if let Err(e) = interp.eval(&script) {
                let report = interp.error_report(&e, &file);
                eprintln!("{}", report);
                std::process::exit(1);
            }
        }
        Outcome::Completed { code, exit_code } => match code {
            0 | 5 => {}
            6 => {
                // exit N：rtcl 文件模式行为（error_report(exit) = to_string）。
                let c = exit_code.unwrap_or(0);
                eprintln!("{}", rtcl_core::Error::exit(Some(c)));
                std::process::exit(1);
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
                std::process::exit(1);
            }
        },
    }
}

fn run_command(cmd: &str) {
    let mut interp = Interp::new();
    let (mut interp, run) = exec::run_top(interp, cmd, "");
    match run.outcome {
        Outcome::Fallback => match interp.eval(cmd) {
            Ok(v) => {
                if !v.is_empty() {
                    println!("{}", v);
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        },
        Outcome::Completed { code, .. } => {
            if code == 0 {
                if let Some(v) = &run.result {
                    if !v.is_empty() {
                        println!("{}", v);
                    }
                }
            } else if let Some(e) = run.eval_error {
                eprintln!("{}", e);
                std::process::exit(1);
            } else if let Some(r) = run.driver_report {
                if !r.is_empty() {
                    eprintln!("{}", r);
                }
                std::process::exit(1);
            }
        }
    }
}
