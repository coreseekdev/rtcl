//! Spencer Rust 引擎微基准（性能结构评估用）。
//!
//! 运行：
//!   cargo run -q -p rtcl-vm --features regexp-spencer --release \
//!     --example spencer_bench
//!
//! 对同一 1 MiB 文本比较：Spencer 单趟 NFA 扫描 vs Rust `regex` crate
//!（fast 引擎）的 `find`。用于决定是否继续上 color/CNFA/lazy DFA。

use rtcl_vm::interp::commands::regexp_spencer as sp;
use std::time::{Duration, Instant};

fn bench<F: FnMut() -> bool>(mut f: F) -> Duration {
    // 预热。
    let _ = f();
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let _ = f();
        let d = t.elapsed();
        if d < best {
            best = d;
        }
    }
    best
}

fn main() {
    let unit = "abab abab abab abab abab abab abab abab\n";
    let mut text = String::with_capacity(1 << 20);
    while text.len() < (1 << 20) {
        text.push_str(unit);
    }
    let bytes = text.len();
    let patterns = [
        "a+",
        "[a-z]+",
        r"\w+",
        "(a|ab)+",
        "(a+?)(a*)",
        "(a|b)*z",
        "[a-z]*z",
        "abab abab",
        r"\s+",
        "$",
    ];
    println!("text = {} bytes", bytes);
    print!("{:<14} {:>10} {:>10} {:>8}", "pattern", "spencer", "regex", "ratio");
    println!();
    for pat in patterns {
        let prog = match sp::compile(pat, sp::Options::default()) {
            Ok(p) => p,
            Err(e) => {
                println!("{:<14} compile error: {}", pat, e.message);
                continue;
            }
        };
        let chars: Vec<char> = text.chars().collect();
        // 引擎核心（字符切片，排除 UTF-8→char 转换）；复用 Searcher
        // 以保留 lazy DFA 缓存，扫描全部匹配。
        let mut searcher = sp::matcher::Searcher::new(&prog);
        let spencer = bench(|| {
            let mut pos = 0usize;
            let mut n = 0usize;
            while pos <= chars.len() {
                match searcher.find(&chars, pos, pos) {
                    Some(m) => {
                        n += 1;
                        pos = if m.end > m.start { m.end } else { m.end + 1 };
                    }
                    None => break,
                }
            }
            n > 0
        });
        let re = match regex::Regex::new(pat) {
            Ok(r) => r,
            Err(e) => {
                println!("{:<14} regex error: {}", pat, e);
                continue;
            }
        };
        let fast = bench(|| {
            let mut n = 0usize;
            for _ in re.find_iter(&text) {
                n += 1;
            }
            n > 0
        });
        let ratio = spencer.as_secs_f64() / fast.as_secs_f64().max(f64::MIN_POSITIVE);
        let mbs = |d: Duration| bytes as f64 / d.as_secs_f64() / (1024.0 * 1024.0);
        println!(
            "{:<14} {:>8.1}MB/s {:>8.1}MB/s {:>7.2}x",
            pat,
            mbs(spencer),
            mbs(fast),
            ratio
        );
        let conv = bench(|| {
            let v: Vec<char> = text.chars().collect();
            v.len() > 0
        });
        println!(
            "{:<14} utf8->chars {:>8.1}MB/s",
            "",
            bytes as f64 / conv.as_secs_f64() / (1024.0 * 1024.0)
        );
    }
}
