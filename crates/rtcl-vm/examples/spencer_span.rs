//! 开发/验收工具：用 Spencer Rust 引擎输出整体匹配 span。
//!
//! 输入（stdin）：每行 `pattern<TAB>text_hex`（text_hex 为 UTF-8 十六进制）。
//! 输出：每行 `start end` 或 `NONE`。供 `specs/regex/` 的差分脚本
//! 与 tclsh `regexp -indices` 对比（M2 整体匹配验收）。
//!
//! 运行：cargo run -p rtcl-vm --features regexp-spencer --example spencer_span

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
            Ok(prog) => match sp::find(&prog, &text, 0) {
                Some(m) => writeln!(out, "{} {}", m.start, m.end).ok(),
                None => writeln!(out, "NONE").ok(),
            },
            Err(_) => writeln!(out, "ERR").ok(),
        };
    }
}
