//! RTCL_DBG 逐写点插桩（诊断专用，**非常驻**）。
//!
//! 编译门：`--features rtcl-dbg` 才有实现；默认构建下 [`event`] 是空
//! `#[inline]` 函数，被编译器完全剔除——release 零常驻开销。
//! 运行门：env `RTCL_DBG` 非空才启用（任何值都累计聚合计数，进程退出
//! 时打印 `T <kind> <count>` 汇总；`=seq` 另打印前 `RTCL_DBG_SEQ_CAP`
//! 条（默认 20000）`E <kind> <detail>` 事件流——形状级逐迭代序列 diff
//! 用；`=seq@N` 跳过前 N 条事件后再开始打印（定位循环中段）。
//!
//! 写点清单（每处一次 `event` 调用）：
//! - `flush<n>` / `invalidate` / `inv1` / `fill` —— ts 同步面（vars.rs）
//! - `fast_ok` / `fast_none` —— incr_var_fast 快径命中/回落（单咽喉）
//! - `cmd_incr` —— 真 incr 全语义入口（commands/misc.rs）
//! - `dispatch` —— dispatch_site 咽喉（每次编译内分派）
//! - `eval_expr` / `eval` —— Interp::eval_expr / Interp::eval 入口
//! - `unit` —— exec_bytecode 单元进入（重编译/重进循环放大器）
//! - `recompile_body` / `recompile_eval` —— F3 失配重编译（proc 体 /
//!   eval 单元）发生点（每单元每世代一次）
//! - `tf_proc` / `tf_eval` —— 树遍历回落点（call_proc 体 / eval_impl
//!   单元；F3/F5/F6 落地后常规脚本应零事件）
//! - `get_var` / `set_var` / `read_top` —— 名路径读写面（vars.rs）
//! - `sub_end` / `foreach_next` / `for_next` —— 区域/迭代标记

/// One write-point event.  Zero-cost when the `rtcl-dbg` feature is off.
#[inline]
pub fn event(kind: &str, detail: &str) {
    #[cfg(feature = "rtcl-dbg")]
    {
        record(kind, detail);
    }
    #[cfg(not(feature = "rtcl-dbg"))]
    {
        let _ = (kind, detail);
    }
}

#[cfg(feature = "rtcl-dbg")]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub struct Dbg {
        pub enabled: bool,
        pub seq: bool,
        pub seq_skip: u64,
        pub seq_cap: u64,
        pub emitted: u64,
        pub seen: u64,
        pub counts: HashMap<String, u64>,
        pub order: Vec<String>,
    }

    impl Dbg {
        fn new() -> Dbg {
            let raw = std::env::var("RTCL_DBG").unwrap_or_default();
            let enabled = !raw.is_empty();
            let (seq, seq_skip) = match raw.strip_prefix("seq") {
                Some(rest) if !rest.is_empty() => {
                    let skip = rest
                        .strip_prefix('@')
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                    (true, skip)
                }
                _ => (raw == "seq", 0),
            };
            let seq_cap = std::env::var("RTCL_DBG_SEQ_CAP")
                .ok()
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(20000);
            Dbg { enabled, seq, seq_skip, seq_cap, emitted: 0, seen: 0, counts: HashMap::new(), order: Vec::new() }
        }

        fn bump(&mut self, key: &str) {
            match self.counts.get_mut(key) {
                Some(c) => *c += 1,
                None => {
                    self.counts.insert(key.to_string(), 1);
                    self.order.push(key.to_string());
                }
            }
        }
    }

    impl Drop for Dbg {
        fn drop(&mut self) {
            if !self.enabled {
                return;
            }
            eprintln!("== RTCL_DBG totals (events={} seq_emitted={}) ==", self.seen, self.emitted);
            let mut keys = self.order.clone();
            keys.sort();
            for k in keys {
                eprintln!("T {} {}", k, self.counts[&k]);
            }
        }
    }

    thread_local! {
        pub static DBG: RefCell<Dbg> = RefCell::new(Dbg::new());
    }

    #[cold]
    pub fn record(kind: &str, detail: &str) {
        DBG.with(|d| {
            let mut d = d.borrow_mut();
            if !d.enabled {
                return;
            }
            d.seen += 1;
            let key = if detail.is_empty() { kind.to_string() } else { format!("{kind}:{detail}") };
            d.bump(&key);
            if d.seq && d.seen > d.seq_skip && d.emitted < d.seq_cap {
                d.emitted += 1;
                if detail.is_empty() {
                    eprintln!("E {}", kind);
                } else {
                    eprintln!("E {} {}", kind, detail);
                }
            }
        });
    }
}

#[cfg(feature = "rtcl-dbg")]
use imp::record;
