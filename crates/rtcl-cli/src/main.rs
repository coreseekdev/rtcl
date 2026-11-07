//! rtcl - A lightweight Tcl-compatible scripting language

use clap::Parser;
use colored::Colorize;
use rtcl_core::Interp;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::path::PathBuf;

mod repl;

// D-40 后续的剖析基建（bench/profile-heap.sh）：jemalloc 全局分配器，
// 仅在 --features jemalloc 且 Linux 时启用。profiling feature 让
// jemalloc 以 --enable-prof 构建——运行期 MALLOC_CONF="prof:true,..."
// 即可出堆快照，jeprof 折叠后接 FlameGraph。默认构建零影响。
#[cfg(all(target_os = "linux", feature = "jemalloc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[derive(Parser, Debug)]
#[command(name = "rtcl", about, version)]
struct Args {
    /// Script file to execute
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Evaluate script string
    #[arg(short = 'c', long)]
    command: Option<String>,

    /// Interactive mode (REPL)
    #[arg(short = 'i', long)]
    interactive: bool,

    /// Quiet mode (don't print banner)
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Run on the tree-walk engine only (bytecode off) — the CLI twin of
    /// the RTCL_NO_BYTECODE env var; a flag leaves the process env, and
    /// thus $env(*), untouched (env enumerations stay comparable).
    #[arg(long)]
    no_bytecode: bool,
}

fn main() {
    // 可选 JIT（--features jit）：proc-body fast-tier 直呼。fast-only
    // 策略——命中 fast 形状才 JIT，其余形状保持解释器。
    #[cfg(feature = "jit")]
    {
        rtcl_jit::proc_exec::install();
        rtcl_jit::proc_exec::set_fast_only(true);
    }
    let args = Args::parse();

    if args.no_bytecode {
        rtcl_core::interp::set_bytecode_disabled(true);
    }

    let result = if let Some(ref file) = args.file {
        run_file(file, args.quiet).map(|()| 0)
    } else if let Some(ref cmd) = args.command {
        run_command(cmd, args.quiet).map(|()| 0)
    } else if args.interactive {
        run_repl(args.quiet)
    } else {
        // Default: show help
        print_help();
        Ok(0)
    };

    match result {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{}: {}", "Error".red(), e);
            }
            std::process::exit(1);
        }
    }
}

fn run_file(path: &PathBuf, quiet: bool) -> Result<(), String> {
    let mut interp = Interp::new();

    // Set script name for info script command
    interp.set_script_name(&path.to_string_lossy());

    let script = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read file '{}': {}", path.display(), e))?;

    let result = interp.eval(&script);

    if let Err(e) = result {
        // 顶层控制流解码（tclsh main 语义，D-11）：先按 -code 解码——
        // `return -code error/break/continue` 与裸 break/continue 在顶层
        // 就是错误（完整 errorInfo rc 1，帧为看到该完成的命令，tclsh
        // 转换点语义），普通 return（含带值）仍是正常完成（rc 0、静默），
        // `exit N` 以 N 退出且无输出。
        let e = interp.decode_toplevel_flow(e);
        if e.is_exit() {
            std::process::exit(repl::exit_code_of(&e));
        }
        if e.is_return() {
            let _ = quiet;
            return Ok(());
        }
        // tclsh prints the full accumulated errorInfo (message + frames +
        // `(file ...)` tail) verbatim on stderr.
        let report = interp.error_report(&e, &path.to_string_lossy());
        eprintln!("{}", report);
        std::process::exit(1);
    }

    let _ = quiet;

    Ok(())
}

fn run_command(cmd: &str, quiet: bool) -> Result<(), String> {
    let mut interp = Interp::new();

    // -c is eval-style interpretation (incremental), unlike -f's compiled
    // whole-script run.
    let result = match interp.eval_incremental(cmd) {
        Ok(r) => r,
        // -c 与 -f 同语义（error-deep 探针：tclsh `catch {exit 3}` 穿透
        // catch 以 3 退出——原 to_string() 把 exit 信号降级成错误文本）。
        Err(e) if e.is_exit() => std::process::exit(repl::exit_code_of(&e)),
        Err(e) if e.is_return() => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };

    if !quiet && !result.is_empty() {
        println!("{}", result);
    }

    Ok(())
}

fn run_repl(quiet: bool) -> Result<i32, String> {
    if !quiet {
        print_banner();
    }

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .build();

    let mut rl: Editor<(), DefaultHistory> = Editor::with_config(config)
        .map_err(|e| format!("failed to create editor: {}", e))?;

    let mut interp = Interp::new();

    // tclsh's interactive main loop runs with tcl_interactive = 1.
    let _ = interp.eval("set tcl_interactive 1");

    let mut input = repl::ReplInput::new();

    loop {
        match rl.readline(repl::prompt_for(input.buffer())) {
            Ok(line) => {
                if input.is_empty() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // rtcl extensions: dot commands. Single-line, primary
                    // prompt only (they are never Tcl input).
                    match trimmed {
                        ".exit" | ".quit" => return Ok(0),
                        ".help" => {
                            print_help();
                            continue;
                        }
                        ".vars" => {
                            show_vars(&mut interp);
                            continue;
                        }
                        ".commands" => {
                            show_commands(&mut interp);
                            continue;
                        }
                        ".history" => {
                            show_history(&rl);
                            continue;
                        }
                        _ => {}
                    }
                }

                // Multi-line read: keep swallowing lines until braces, quotes
                // and brackets balance out, exactly like tclsh's interactive
                // loop (docs/REPL-TCLSH-PARITY.md).
                match input.feed(&line) {
                    None => continue,
                    Some(script) => {
                        let _ = rl.add_history_entry(&script);
                        match eval_repl_line(&mut interp, &script) {
                            Ok(()) => {}
                            Err(code) => return Ok(code), // top-level exit N
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // tclsh 8.6 dies on SIGINT; staying alive and aborting the
                // current input is a deliberate rtcl divergence.
                println!("^C");
                input.clear();
            }
            Err(ReadlineError::Eof) => {
                // tclsh exits silently on EOF (Ctrl-D).
                return Ok(0);
            }
            Err(e) => {
                eprintln!("readline error: {:?}", e);
                return Ok(1);
            }
        }
    }
}

/// Evaluate one completed script and print tclsh-style interactive output:
/// non-empty results on stdout, bare error messages on stderr (no prefix —
/// tclsh interactive prints the message only; the full errorInfo stack is
/// script-mode behavior, see run_file). `Err(code)` is a top-level `exit`
/// carrying its process exit code.
fn eval_repl_line(interp: &mut Interp, script: &str) -> Result<(), i32> {
    match interp.eval_incremental(script) {
        Ok(result) => {
            if !result.is_empty() {
                println!("{}", result);
            }
            Ok(())
        }
        Err(e) => {
            // 顶层控制流解码（tclsh main 语义，D-11）：return -code
            // error/break/continue 与裸 break/continue 是错误（REPL 侧
            // 裸消息呈现、继续循环）；普通 return 仍走 control_flow_text
            // 回显。exit 在解码中穿透。
            let e = interp.decode_toplevel_flow(e);
            if e.is_exit() {
                return Err(repl::exit_code_of(&e));
            }
            if let Some(text) = repl::control_flow_text(&e) {
                // `return ok` echoes like a result; break/continue report
                // through the tclsh exception text.
                if !text.is_empty() {
                    if e.is_return() {
                        println!("{}", text);
                    } else {
                        eprintln!("{}", text);
                    }
                }
                return Ok(());
            }
            eprintln!("{}", e);
            Ok(())
        }
    }
}

fn print_banner() {
    println!("{}", "rtcl - Lightweight Tcl-compatible interpreter".green().bold());
    println!("Version: {}", rtcl_core::VERSION);
    println!("Type {} for help, {} to quit", ".help".cyan(), ".exit".cyan());
    println!();
}

fn print_help() {
    println!("{}", "Tcl Commands:".bold());
    println!("  set <var> <value>          Set a variable");
    println!("  puts <string>              Print a string");
    println!("  if <expr> {{body}}         Conditional");
    println!("  while <expr> {{body}}      While loop");
    println!("  for <init> <test> <next>   For loop");
    println!("  foreach <var> <list> {{b}} Foreach loop");
    println!("  expr <expression>          Evaluate expression");
    println!("  proc <name> <args> {{body}} Define procedure");
    println!("  return <value>             Return from procedure");
    println!("  break / continue           Loop control");
    println!();
    println!("{}", "List Commands:".bold());
    println!("  list <items...>            Create a list");
    println!("  llength <list>             Get list length");
    println!("  lindex <list> <i>          Get list element");
    println!("  lappend <var> <items...>   Append to list");
    println!();
    println!("{}", "String Commands:".bold());
    println!("  string length <s>          String length");
    println!("  string range <s> <a> <b>   Substring");
    println!("  string tolower/toupper     Case conversion");
    println!();
    println!("{}", "Other Commands:".bold());
    println!("  incr <var> <n>             Increment variable");
    println!("  catch {{script}} <var>     Catch errors");
    println!("  info exists <var>          Check variable");
    println!("  info commands              List commands");
    println!();
    println!("{}", "REPL Commands:".bold());
    println!("  .help                      Show this help");
    println!("  .exit / .quit              Exit REPL");
    println!("  .vars                      List global variables");
    println!("  .commands                  List commands");
    println!("  .history                   Show input history");
    println!();
}

fn show_vars(interp: &mut Interp) {
    match interp.eval("lsort [info globals]") {
        Ok(v) => println!("{}", v),
        Err(e) => eprintln!("{}", e),
    }
}

fn show_commands(interp: &mut Interp) {
    match interp.eval("lsort [info commands]") {
        Ok(v) => println!("{}", v),
        Err(e) => eprintln!("{}", e),
    }
}

fn show_history(rl: &Editor<(), DefaultHistory>) {
    for (i, entry) in rl.history().iter().enumerate() {
        println!("{: >5}  {}", i + 1, entry);
    }
}
