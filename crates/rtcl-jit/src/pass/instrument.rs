//! `instrument` — L3→L3：回边 fuel 自减、epoch 比对织入（设计文档 §3）。
//! lower 产出的每个 `backedge=true` 的 Br/BrIf 前插入：
//! `BackedgeFuel`（fuel 自减、耗尽调 `rtcl.refuel`、哨兵检查）+
//! `EpochCheck`（回边比对，不等 → 返回 STALE）。纯织入，不改结构。

use crate::ir::{Instr, L3};
use crate::pass::EmitConfig;

pub fn run(mut l3: L3, cfg: &EmitConfig) -> L3 {
    let _ = cfg;
    weave(&mut l3);
    l3
}

fn weave(node: &mut L3) {
    match node {
        L3::Seq(items) => {
            let mut i = 0;
            while i < items.len() {
                let back = match &items[i] {
                    L3::Br { backedge, .. } | L3::BrIf { backedge, .. } if *backedge => true,
                    _ => false,
                };
                if back {
                    let fail = match &items[i] {
                        L3::Br { fail, .. } | L3::BrIf { fail, .. } => *fail,
                        _ => 0,
                    };
                    items.insert(i, L3::Instr(Instr::BackedgeFuel { fail }));
                    items.insert(i + 2, L3::Instr(Instr::EpochCheck { at_entry: false }));
                    i += 3;
                } else {
                    weave(&mut items[i]);
                    i += 1;
                }
            }
        }
        L3::If { then_, else_, .. } => {
            weave(then_);
            weave(else_);
        }
        L3::Loop { body, .. } => weave(body),
        L3::Block { body, .. } => weave(body),
        _ => {}
    }
}
