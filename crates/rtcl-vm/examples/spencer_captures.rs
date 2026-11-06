//! 开发/验收工具：Spencer Rust 引擎输出捕获 span（tclsh `-indices -inline`
//! 的闭区间格式：非空 `s e`，空匹配 `s s-1`，未参与 `-1 -1`）。
//!
//! 输入 stdin：每行 `pattern<TAB>text_hex`；输出每行空格分隔的 span 列表
//! 或 `NONE`。

use rtcl_vm::interp::commands::regexp_spencer as sp;
use std::io::{self, BufRead, Write};

fn decode_hex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        bytes.push((hi * 16 + lo) as u8);
        i += 2;
    }
    String::from_utf8(bytes).ok()
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let Some((pat, hex)) = line.split_once('\t') else {
            writeln!(out, "NONE").ok();
            continue;
        };
        let text = decode_hex(hex).unwrap_or_default();
        match sp::compile(pat, sp::Options::default()) {
            Ok(prog) => match sp::captures(&prog, &text, 0) {
                Some(caps) => {
                    let mut parts = Vec::new();
                    for span in &caps.spans {
                        match span {
                            Some((s, e)) => {
                                let end = if e > s { e - 1 } else { *s };
                                // 空匹配在 Tcl 里 end = start-1。
                                let end = if e == s { (*s as i64) - 1 } else { end as i64 };
                                parts.push(format!("{} {}", s, end));
                            }
                            None => parts.push("-1 -1".to_string()),
                        }
                    }
                    writeln!(out, "{}", parts.join(" ")).ok();
                }
                None => {
                    writeln!(out, "NONE").ok();
                }
            },
            Err(_) => {
                writeln!(out, "ERR").ok();
            }
        }
    }
}
