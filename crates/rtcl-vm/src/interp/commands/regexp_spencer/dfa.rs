//! Lazy DFA 缓存（Spencer `rege_dfa.c` 的 Rust 版语义结构）。
//!
//! * **颜色类**：收集 NFA 中不同的 CharSet，每个字符映射为"命中集合"
//!   位掩码；字符→紧凑颜色 id（ASCII 直表，Unicode 惰性缓存）。
//! * **DFA 状态**：NFA 状态位集（Vec<u64>），eps 闭包后规范化去重；
//!   转移表 = 每状态一行 `Vec<u32>`（按颜色 id 下标，u32::MAX 空）。
//! * **限制**：断言（^ $ \A ...）依赖位置，含断言时禁用 DFA 回退 NFA；
//!   状态数超过上限返回 Err（调用方回退，防内存膨胀）。

use std::collections::HashMap;

use super::charset::CharSet;
use super::nfa::{Prog, Trans};

const MAX_STATES: usize = 4096;
const MAX_COLORS: usize = 64;
const EMPTY: u32 = u32::MAX;
const DEAD: u32 = u32::MAX - 1;

struct ColorInfo {
    sets: Vec<CharSet>,
    icase: bool,
    /// NFA 状态 i 的字符弧 (集合位, 目标 NFA 状态)。
    arcs: Vec<Vec<(u64, usize)>>,
    /// 不同掩码 → 紧凑颜色 id。
    masks: Vec<u64>,
    mask_ids: HashMap<u64, u32>,
    /// 0..255 的颜色 id 快表。
    ascii: [u32; 256],
    /// >255 的 char → 颜色 id 缓存。
    cache: HashMap<u32, u32>,
}

impl ColorInfo {
    fn build(prog: &Prog) -> Option<ColorInfo> {
        let mut sets: Vec<CharSet> = Vec::new();
        let mut arcs: Vec<Vec<(u64, usize)>> = vec![Vec::new(); prog.states.len()];
        for (si, st) in prog.states.iter().enumerate() {
            for t in &st.trans {
                match t {
                    Trans::Char(set, to) => {
                        let idx = match sets.iter().position(|s| s == set) {
                            Some(i) => i,
                            None => {
                                if sets.len() >= MAX_COLORS {
                                    return None;
                                }
                                sets.push(set.clone());
                                sets.len() - 1
                            }
                        };
                        arcs[si].push((1u64 << idx, *to));
                    }
                    Trans::Assert(_, _) => return None,
                    Trans::Eps(_) => {}
                }
            }
        }
        if sets.is_empty() {
            return None;
        }
        let mut info = ColorInfo {
            sets,
            icase: prog.icase,
            arcs,
            masks: Vec::new(),
            mask_ids: HashMap::new(),
            ascii: [0u32; 256],
            cache: HashMap::new(),
        };
        for u in 0..256u32 {
            let c = char::from_u32(u).unwrap();
            info.ascii[u as usize] = info.compute_id(c);
        }
        Some(info)
    }

    fn compute_id(&mut self, c: char) -> u32 {
        let mut mask = 0u64;
        for (i, set) in self.sets.iter().enumerate() {
            if set.matches(c, self.icase) {
                mask |= 1u64 << i;
            }
        }
        if let Some(&id) = self.mask_ids.get(&mask) {
            return id;
        }
        let id = self.masks.len() as u32;
        self.masks.push(mask);
        self.mask_ids.insert(mask, id);
        id
    }

    fn id(&mut self, c: char) -> u32 {
        let u = c as u32;
        if u < 256 {
            return self.ascii[u as usize];
        }
        if let Some(&id) = self.cache.get(&u) {
            return id;
        }
        let id = self.compute_id(c);
        self.cache.insert(u, id);
        id
    }
}

struct DfaState {
    bits: Vec<u64>,
    accept: bool,
}

pub struct DfaCache {
    colors: ColorInfo,
    states: Vec<DfaState>,
    /// 每状态一行：颜色 id → 后继状态（EMPTY 未计算）。
    trans: Vec<Vec<u32>>,
    index: HashMap<Vec<u64>, u32>,
    start: u32,
}

impl DfaCache {
    pub fn new(prog: &Prog) -> Option<DfaCache> {
        let colors = ColorInfo::build(prog)?;
        let mut cache = DfaCache {
            colors,
            states: Vec::new(),
            trans: Vec::new(),
            index: HashMap::new(),
            start: 0,
        };
        let mut bits = vec![0u64; (prog.states.len() + 63) / 64];
        set_bit(&mut bits, prog.start);
        eps_closure(prog, &mut bits);
        let start = cache.intern(prog, bits)?;
        cache.start = start;
        Some(cache)
    }

    pub fn start(&self) -> u32 {
        self.start
    }

    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    pub fn is_accept(&self, id: u32) -> bool {
        self.states[id as usize].accept
    }

    pub fn color(&mut self, c: char) -> u32 {
        self.colors.id(c)
    }

    /// 按颜色 id 转移；`Ok(None)` 死状态，`Err(())` 状态超限。
    pub fn step(&mut self, prog: &Prog, id: u32, color: u32) -> Result<Option<u32>, ()> {
        let row = &mut self.trans[id as usize];
        if row.len() <= color as usize {
            row.resize(color as usize + 1, EMPTY);
        }
        let cached = row[color as usize];
        if cached == DEAD {
            return Ok(None);
        }
        if cached != EMPTY {
            return Ok(Some(cached));
        }
        let mask = self.colors.masks[color as usize];
        let bits = &self.states[id as usize].bits;
        let mut next = vec![0u64; bits.len()];
        let mut any = false;
        for (wi, &word) in bits.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let b = w.trailing_zeros() as usize;
                w &= w - 1;
                let si = wi * 64 + b;
                for &(bit, to) in &self.colors.arcs[si] {
                    if mask & bit != 0 {
                        set_bit(&mut next, to);
                        any = true;
                    }
                }
            }
        }
        if !any {
            self.trans[id as usize][color as usize] = DEAD;
            return Ok(None);
        }
        eps_closure(prog, &mut next);
        let to = self.intern(prog, next).ok_or(())?;
        self.trans[id as usize][color as usize] = to;
        Ok(Some(to))
    }

    fn intern(&mut self, prog: &Prog, bits: Vec<u64>) -> Option<u32> {
        if let Some(&id) = self.index.get(&bits) {
            return Some(id);
        }
        if self.states.len() >= MAX_STATES {
            return None;
        }
        let accept = get_bit(&bits, prog.accept);
        let id = self.states.len() as u32;
        self.states.push(DfaState {
            bits: bits.clone(),
            accept,
        });
        self.trans.push(vec![EMPTY; self.colors.masks.len()]);
        self.index.insert(bits, id);
        Some(id)
    }
}

fn set_bit(bits: &mut [u64], i: usize) {
    bits[i / 64] |= 1u64 << (i % 64);
}

fn get_bit(bits: &[u64], i: usize) -> bool {
    bits[i / 64] & (1u64 << (i % 64)) != 0
}

fn eps_closure(prog: &Prog, bits: &mut [u64]) {
    let mut stack: Vec<usize> = Vec::new();
    for (wi, &word) in bits.iter().enumerate() {
        let mut w = word;
        while w != 0 {
            let b = w.trailing_zeros() as usize;
            w &= w - 1;
            stack.push(wi * 64 + b);
        }
    }
    while let Some(s) = stack.pop() {
        for t in &prog.states[s].trans {
            if let Trans::Eps(to) = t {
                let already = get_bit(bits, *to);
                set_bit(bits, *to);
                if !already {
                    stack.push(*to);
                }
            }
        }
    }
}
