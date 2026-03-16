//! rtcl - A lightweight Tcl-compatible scripting language

use annotate_snippets::{Level, Renderer, Snippet};
use clap::Parser;
use colored::Colorize;
use rtcl_core::Interp;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::path::PathBuf;

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
}

fn main() {
    let args = Args::parse();

    let result = if let Some(ref file) = args.file {
        run_file(file, args.quiet)
    } else if let Some(ref cmd) = args.command {
        run_command(cmd, args.quiet)
    } else if args.interactive {
        run_repl(args.quiet)
    } else {
        // Default: show help
        print_help();
        Ok(())
    };

    if let Err(e) = result {
        if !e.is_empty() {
            eprintln!("{}: {}", "Error".red(), e);
        }
        std::process::exit(1);
    }
}

fn run_file(path: &PathBuf, quiet: bool) -> Result<(), String> {
    let mut interp = Interp::new();

    // Set script name for info script command
    interp.set_script_name(&path.to_string_lossy());

    let script = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read file '{}': {}", path.display(), e))?;

    // Pre-parse for rich diagnostics on syntax errors
    if let Err(e) = rtcl_parser::parse(&script) {
        report_parse_error(&e, &script, Some(&path.to_string_lossy()));
        return Err(String::new());
    }

    let result = interp.eval(&script);

    if let Err(e) = result {
        return Err(e.to_string());
    }

    if !quiet {
        println!("Script executed successfully");
    }

    Ok(())
}

fn run_command(cmd: &str, quiet: bool) -> Result<(), String> {
    let mut interp = Interp::new();

    // Pre-parse for rich diagnostics on syntax errors
    if let Err(e) = rtcl_parser::parse(cmd) {
        report_parse_error(&e, cmd, None);
        return Err(String::new());
    }

    let result = interp.eval(cmd)
        .map_err(|e| e.to_string())?;

    if !quiet && !result.is_empty() {
        println!("{}", result);
    }

    Ok(())
}

fn run_repl(quiet: bool) -> Result<(), String> {
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

    loop {
        match rl.readline("rtcl> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line.to_owned());

                // Special commands
                match line {
                    ".exit" | ".quit" => break,
                    ".help" => {
                        print_help();
                        continue;
                    }
                    ".vars" => {
                        show_vars(&interp);
                        continue;
                    }
                    ".commands" => {
                        show_commands();
                        continue;
                    }
                    _ => {}
                }

                // Evaluate — pre-parse for rich diagnostics
                if let Err(e) = rtcl_parser::parse(line) {
                    report_parse_error(&e, line, None);
                    continue;
                }
                match interp.eval(line) {
                    Ok(result) => {
                        if !result.is_empty() {
                            println!("{}", result);
                        }
                    }
                    Err(e) => {
                        // Handle control flow (not real errors in REPL)
                        if e.is_break() || e.is_continue() || e.is_return() {
                            println!("{}", "Warning: control flow outside loop/procedure".yellow());
                            continue;
                        }
                        eprintln!("{}: {}", "Error".red(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Ctrl-D");
                break;
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Render a parse error using rustc-style annotated snippets.
fn report_parse_error(err: &rtcl_parser::ParseError, source: &str, filename: Option<&str>) {
    let origin = filename.unwrap_or("<input>");

    // Build a 1-byte span at the error offset (or end of source)
    let offset = err.offset.min(source.len());
    let span_end = if offset < source.len() { offset + 1 } else { offset };

    let message = Level::Error.title(&err.message).snippet(
        Snippet::source(source)
            .line_start(1)
            .origin(origin)
            .fold(true)
            .annotation(Level::Error.span(offset..span_end).label(&err.message)),
    );

    let renderer = Renderer::styled();
    eprintln!("{}", renderer.render(message));
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
    println!("  .vars                      List variables");
    println!("  .commands                  List commands");
    println!();
}

fn show_vars(_interp: &Interp) {
    // This would require exposing variable iteration
    println!("(Variable listing not yet implemented)");
}

fn show_commands() {
    println!("{}", "Built-in Commands:".bold());
    let commands = [
        "set", "puts", "if", "while", "for", "foreach",
        "break", "continue", "return", "proc", "expr",
        "string", "list", "llength", "lindex", "lappend",
        "concat", "append", "incr", "catch", "error",
        "global", "unset", "info", "rename", "eval",
    ];
    for cmd in &commands {
        println!("  {}", cmd);
    }
}
