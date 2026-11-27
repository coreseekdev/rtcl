//! VarArena — D1 变量表重写（specs/ARCH-DESIGN-20261126.md D1）的基础
//! 设施：generation-guarded 句柄 arena + 有序 Var 表。
//!
//! 目标形状（D1 节）：三类表——全局表（每命名空间一张）、帧变量表、
//! 数组元素表——都是真实哈希容器（桶链 + 插入序向量，顺序天然，删除
//! 留洞标记）；表项 = [`Var`]；「解析一次返回句柄」——[`VarRef`] =
//! 表 + 槽 + 代数，之后的读/写/存在/删除/数组/trace 全部对句柄操作。
//!
//! 本文件是纯数据结构层（不依赖 [`super::Interp`]）：
//!
//! - **generation guard**：槽删除只置洞 + 代数 +1，槽地址永不搬移——
//!   `&Var` 指针在表生命期内稳定；空洞槽位可复用（新条目继承递增后
//!   的代数，旧句柄代数不符 → [`VarTable::get_ref`] 返回 None，ABA
//!   防护）。
//! - **有序性**：插入序向量记录槽下标序列，删除置洞标记、枚举跳洞；
//!   「删除后重建的键取新位、纯覆盖保持原位」——与既有平表 + 插入日
//!   志重放（`var_order` + `tclhash::tcl_order`）的顺序语义逐字节一致
//!   （D1 步骤 1 的对拍由 `vars.rs` 的读路径在 debug 下断言）。
//! - **嵌入兼容**：`embedded` 特性下键索引退化为 `BTreeMap`（同
//!   `varmap.rs` 的既有模式），无桶链与哈希器。
//!
//! 注：本批每张 [`VarTable`] 只服务 0 号全局表（`::`，平键域与现
//! `globals` 同域），句柄的 [`TableId::Global`] 下标因此恒 0；步骤 2/3
//! 接入帧/数组元素表时表实例将携带自己的 arena 下标。

#[cfg(not(feature = "embedded"))]
use super::varmap::VarHasher;
#[cfg(not(feature = "embedded"))]
use std::hash::Hasher as _;
#[cfg(not(feature = "embedded"))]
use std::rc::Rc;
#[cfg(feature = "embedded")]
use alloc::boxed::Box;
#[cfg(feature = "embedded")]
use alloc::rc::Rc;
#[cfg(feature = "embedded")]
use alloc::vec;
#[cfg(feature = "embedded")]
use alloc::string::String;
#[cfg(feature = "embedded")]
use alloc::vec::Vec;
#[cfg(not(feature = "embedded"))]
use std::string::String;
#[cfg(not(feature = "embedded"))]
use std::vec::Vec;

use crate::value::Value;

/// 空槽/空桶/序洞哨兵（`u32::MAX`）。
const NO_SLOT: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// 句柄
// ---------------------------------------------------------------------------

/// 表身份（D1：三类表）。步骤 1 只用 `Global(0)`；`Frame`/`Array` 为
/// 步骤 2/3 的占位臂。
#[allow(dead_code)] // 步骤 2/3 接线后移除（Frame/Array 臂占位）
#[derive(Debug, Clone)]
pub(crate) enum TableId {
    /// 全局（命名空间）表：[`VarArena::tables`] 的下标。
    Global(u32),
    /// 帧变量表（步骤 2 迁移；占位）。
    Frame(u32),
    /// 数组元素表（步骤 3 迁移；占位）——基变量的句柄。
    Array(Box<VarRef>),
}

/// 「解析一次」返回的句柄：表 + 槽 + 代数。槽删除后代数递增，持旧
/// 句柄的读取一律 miss（返回 None）——指针稳定与 ABA 防护并存。
/// （句柄 API 的消费面在步骤 2/3 接线——「解析一次」替代平键快径。）
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct VarRef {
    pub table: TableId,
    pub slot: u32,
    pub gen: u32,
}

// ---------------------------------------------------------------------------
// Var
// ---------------------------------------------------------------------------

/// Var 形态位（D1：ARRAY / UNDEFINED / LINKED）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct VarFlags(u8);

impl VarFlags {
    /// 数组容器。步骤 1 语义：全局平键域的数组基标记（镜像
    /// `array_globals`）；帧域数组标记随步骤 2 迁入。
    pub(crate) const ARRAY: Self = Self(1 << 0);
    /// 无值条目（phantom：trace/`global`/`upvar #0` 链接占位——镜像
    /// `phantom_globals`）。与「有值」独立：链接目标先 phantom 后
    /// `set` 的重叠态下两位同置（对拍逐位镜像，vars.rs 双写层维护）。
    pub(crate) const UNDEFINED: Self = Self(1 << 1);
    /// 被链接（步骤 3/4 链接面迁入时启用；本批恒 0）。
    #[allow(dead_code)]
    pub(crate) const LINKED: Self = Self(1 << 2);

    #[inline]
    pub(crate) fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub(crate) fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub(crate) fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

/// trace 集占位（步骤 4：`var_traces`/`elem_traces`/`trace_phantoms`
/// 三表收进 `Var.traces`；本批恒 `None`——不可构造类型使占位零成本）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum TraceSet {}

/// 链接目标占位（步骤 3/4：链接/死链面迁入；本批恒 `None`）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum LinkTarget {}

/// 表项（D1 目标形状）：值 + 形态位 + 存活元素数 + trace/链接占位。
#[derive(Debug, Clone)]
pub(crate) struct Var {
    /// 值。无值条目为 [`Value::unset`] 哨兵——「有值 ⟺ 旧路径
    /// `globals` 含键」独立于 [`VarFlags::UNDEFINED`]（后者只镜像
    /// phantom 成员位）。
    pub value: Value,
    pub flags: VarFlags,
    /// ARRAY 时：存活元素数（T7 `elem_counts` 的 Var 内化——本批全局
    /// 域；帧域随步骤 2/3）。
    pub elem_count: u32,
    /// 变量/元素 trace 集（步骤 4 挂载；本批恒 `None`）。
    #[allow(dead_code)]
    pub traces: Option<TraceSet>,
    /// 链接目标（步骤 3/4 迁入；本批恒 `None`）。
    #[allow(dead_code)]
    pub link: Option<LinkTarget>,
}

impl Var {
    /// 带值的表项（其余位为默认）。
    pub(crate) fn with_value(v: Value) -> Self {
        Self {
            value: v,
            flags: VarFlags::default(),
            elem_count: 0,
            traces: None,
            link: None,
        }
    }

    /// 有值（非 unset 哨兵）。
    #[inline]
    pub(crate) fn has_value(&self) -> bool {
        !self.value.is_unset()
    }

    /// 数组容器。
    #[inline]
    pub(crate) fn is_array(&self) -> bool {
        self.flags.contains(VarFlags::ARRAY)
    }

    /// 无值占位条目（phantom 成员位）。
    #[inline]
    pub(crate) fn is_undefined(&self) -> bool {
        self.flags.contains(VarFlags::UNDEFINED)
    }
}

// ---------------------------------------------------------------------------
// 槽
// ---------------------------------------------------------------------------

/// arena 槽：删除只置洞 + 代数 +1——槽地址永不搬移，`&Var` 指针稳定；
/// 空洞槽位可复用（代数防护旧句柄）。
struct Slot {
    /// 代数：分配时从 0 起；每次 remove +1。句柄命中要求
    /// `ref.gen == gen`；复用槽的新条目继承递增后的代数。
    gen: u32,
    /// 键（`None` = 洞）。
    key: Option<Rc<str>>,
    /// 表项（`None` = 洞；与 `key` 同步生死）。
    var: Option<Var>,
    /// 桶链 next（槽下标；[`NO_SLOT`] = 链尾）。embedded 后备不用。
    #[cfg(not(feature = "embedded"))]
    next: u32,
    /// 插入序向量中的位置（删除时置洞标记用）。
    order_pos: u32,
}

// ---------------------------------------------------------------------------
// VarTable
// ---------------------------------------------------------------------------

/// 一张有序 Var 表：generation-guarded 槽 arena + 桶链键索引 + 插入序
/// 向量。D1 的三类表（全局/帧/数组元素）共用此形状。
///
/// 顺序语义：插入序向量 + 跳洞枚举 = 「每个活键恰一次，在它最后一次
/// 创建的位置」——与 `var_order` 日志的 liveness 过滤 + 保留末次出现
/// （`global_var_order`/`compact_order` 的既有语义）一致，经
/// `tclhash::tcl_order` 重放即 tclsh 的表枚举序。`embedded` 后备：
/// 键索引退化为 `BTreeMap`（无桶链、无哈希器——同 varmap.rs 模式）。
pub(crate) struct VarTable {
    /// 槽 arena（只增；删除置洞，空洞槽位入 free 复用）。
    slots: Vec<Slot>,
    /// 桶链头（槽下标；[`NO_SLOT`] = 空桶）。embedded 后备不用。
    #[cfg(not(feature = "embedded"))]
    buckets: Vec<u32>,
    /// embedded 后备键索引：键 → 槽下标。
    #[cfg(feature = "embedded")]
    index: alloc::collections::BTreeMap<Rc<str>, u32>,
    /// 插入序：槽下标序列；删除置 [`NO_SLOT`] 洞标记，枚举跳洞。
    order: Vec<u32>,
    /// 活条目数。
    live: usize,
    /// 空洞槽复用队列。
    free: Vec<u32>,
    /// ARRAY 标记条目数（`!array_globals.is_empty()` 守卫的 O(1) 等价）。
    arrays: u32,
    /// 数组基快索引：基键 → 槽下标。**访问路径，非平行存储**——ARRAY
    /// 位与元素计数只存在 Var 上，本索引只把它们做成 O(1) 小表探测
    /// （旧 `array_globals`/`elem_counts` 是 1-2 条目的微小集合，探测
    /// 近乎免费；元素平键与基键同表期间（步骤 3 拆元素表前），对 8 万
    /// 元素大表探测基键是 T7 快径的成本回退——本索引保持旧成本形）。
    /// 维护点：mark/unmark/remove（位变化即同步）。
    #[cfg(not(feature = "embedded"))]
    arr_index: super::varmap::VarMap<u32>,
    #[cfg(feature = "embedded")]
    arr_index: alloc::collections::BTreeMap<Rc<str>, u32>,
}

impl Default for VarTable {
    fn default() -> Self {
        Self::new()
    }
}

impl VarTable {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            #[cfg(not(feature = "embedded"))]
            buckets: vec![NO_SLOT; 8],
            #[cfg(feature = "embedded")]
            index: alloc::collections::BTreeMap::new(),
            order: Vec::new(),
            live: 0,
            free: Vec::new(),
            arrays: 0,
            #[cfg(not(feature = "embedded"))]
            arr_index: super::varmap::VarMap::default(),
            #[cfg(feature = "embedded")]
            arr_index: alloc::collections::BTreeMap::new(),
        }
    }

    /// 活条目数。
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.live
    }

    /// 空（无活条目）。
    #[allow(dead_code)] // 表 API 面（测试用；步骤 2/3 接线）
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// ARRAY 标记条目数。
    #[inline]
    pub(crate) fn array_count(&self) -> u32 {
        self.arrays
    }

    /// 数组基是否全为启动数组（env/tcl_platform）——`arrays_env_only`
    /// 重算的 arena 侧真源（旧形：镜像集 `iter().all(...)` 逐字同域）。
    pub(crate) fn arrays_all_startup(&self) -> bool {
        self.arr_index
            .keys()
            .all(|k| k == "env" || k == "tcl_platform")
    }

    /// 数组基标记探测（小表快索引——T7 成本形保持；位真值在 Var 上，
    /// 索引与位同步维护）。
    #[inline]
    pub(crate) fn is_array_base(&self, key: &str) -> bool {
        self.arr_index.contains_key(key)
    }

    /// 数组基的 Var 访问（经快索引 → 槽）。计数读写走这条单探测路径。
    #[inline]
    pub(crate) fn array_var(&self, key: &str) -> Option<&Var> {
        let &sidx = self.arr_index.get(key)?;
        Some(self.slots[sidx as usize].var.as_ref().expect("indexed slot"))
    }

    #[inline]
    pub(crate) fn array_var_mut(&mut self, key: &str) -> Option<&mut Var> {
        let sidx = *self.arr_index.get(key)?;
        Some(self.slots[sidx as usize].var.as_mut().expect("indexed slot"))
    }

    /// 数组基标记（位 + 计数；快索引对账）。无条目时建无值 Var
    /// （启动窗口：标记先于基键落表——插入序红线不动）。已标记时只
    /// 更新计数。
    pub(crate) fn mark_array(&mut self, key: &str, count: Option<u32>) {
        if let Some(&sidx) = self.arr_index.get(key) {
            if let Some(n) = count {
                self.slots[sidx as usize]
                    .var
                    .as_mut()
                    .expect("indexed slot")
                    .elem_count = n;
            }
            return;
        }
        match self.slot_of(key) {
            Some(sidx) => {
                let was = {
                    let var = self.slots[sidx as usize].var.as_mut().expect("live slot");
                    let was = var.is_array();
                    var.flags.insert(VarFlags::ARRAY);
                    if let Some(n) = count {
                        var.elem_count = n;
                    }
                    was
                };
                if !was {
                    self.arrays += 1;
                    self.arr_index.insert(key.to_string(), sidx);
                }
            }
            None => {
                let mut var = Var::with_value(Value::unset());
                var.flags.insert(VarFlags::ARRAY);
                if let Some(n) = count {
                    var.elem_count = n;
                }
                // 走 insert（ARRAY 条目入表时快索引自动对账）。
                self.insert(Rc::from(key), var);
            }
        }
    }

    /// 数组基摘标（计数随位同清；快索引对账）。
    pub(crate) fn unmark_array(&mut self, key: &str) {
        if let Some(&sidx) = self.arr_index.get(key) {
            let slot = &mut self.slots[sidx as usize];
            let var = slot.var.as_mut().expect("indexed slot");
            var.flags.remove(VarFlags::ARRAY);
            var.elem_count = 0;
            self.arrays -= 1;
            self.arr_index.remove(key);
        }
    }

    /// 是否持有该键的条目（有值或占位皆算——「表里有这条 Var」）。
    #[allow(dead_code)] // 表 API 面（测试用；步骤 2/3 接线）
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.slot_of(key).is_some()
    }

    // -- 键索引 ------------------------------------------------------------

    /// 键 → 槽下标（仅活条目）。
    #[cfg(not(feature = "embedded"))]
    fn slot_of(&self, key: &str) -> Option<u32> {
        let mut idx = self.buckets[bucket_of(key, self.buckets.len())];
        while idx != NO_SLOT {
            let slot = &self.slots[idx as usize];
            if slot.key.as_deref() == Some(key) {
                return Some(idx);
            }
            idx = slot.next;
        }
        None
    }

    #[cfg(feature = "embedded")]
    fn slot_of(&self, key: &str) -> Option<u32> {
        self.index.get(key).copied()
    }

    /// 槽下标 → VarRef（代数入柄）。
    fn ref_of(&self, idx: u32) -> VarRef {
        VarRef {
            table: TableId::Global(0),
            slot: idx,
            gen: self.slots[idx as usize].gen,
        }
    }

    // -- 读写 ----------------------------------------------------------------

    /// 插入/覆盖。已存在的键：原地替换值，**保持槽位与插入序位置**
    /// （tclsh 哈希条目语义：纯 re-set 不动位置）；新键：分配槽（复用
    /// 空洞或追加）、桶链头插、插入序追加（删除后重建的键取新位）。
    /// 返回句柄。
    pub(crate) fn insert(&mut self, key: Rc<str>, var: Var) -> VarRef {
        #[cfg(not(feature = "embedded"))]
        {
            let bidx = bucket_of(&key, self.buckets.len());
            let mut idx = self.buckets[bidx];
            while idx != NO_SLOT {
                if self.slots[idx as usize].key.as_deref() == Some(&*key) {
                    let new_is_array = var.is_array();
                    let was_array = {
                        let slot = &mut self.slots[idx as usize];
                        let was = slot.var.as_ref().is_some_and(|v| v.is_array());
                        slot.var = Some(var);
                        was
                    };
                    if was_array != new_is_array {
                        self.arrays += u32::from(new_is_array);
                        self.arrays -= u32::from(was_array);
                        self.reconcile_arr_index(&key, idx, was_array);
                    }
                    return self.ref_of(idx);
                }
                idx = self.slots[idx as usize].next;
            }
            let sidx = self.alloc_slot(key.clone(), bidx);
            let is_array = {
                let slot = &mut self.slots[sidx as usize];
                slot.var = Some(var);
                slot.var.as_ref().is_some_and(|v| v.is_array())
            };
            if is_array {
                self.arrays += 1;
                self.arr_index.insert(key.as_ref().to_string(), sidx);
            }
            self.order.push(sidx);
            self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
            self.live += 1;
            self.maybe_grow_buckets();
            self.maybe_compact_order();
            self.ref_of(sidx)
        }
        #[cfg(feature = "embedded")]
        {
            match self.index.get(&key).copied() {
                Some(sidx) => {
                    let new_is_array = var.is_array();
                    let was_array = {
                        let slot = &mut self.slots[sidx as usize];
                        let was = slot.var.as_ref().is_some_and(|v| v.is_array());
                        slot.var = Some(var);
                        was
                    };
                    if was_array != new_is_array {
                        self.arrays += u32::from(new_is_array);
                        self.arrays -= u32::from(was_array);
                        self.reconcile_arr_index(&key, sidx, was_array);
                    }
                    VarRef { table: TableId::Global(0), slot: sidx, gen: self.slots[sidx as usize].gen }
                }
                None => {
                    let sidx = self.alloc_slot(key.clone());
                    let is_array = {
                        let slot = &mut self.slots[sidx as usize];
                        slot.var = Some(var);
                        slot.var.as_ref().is_some_and(|v| v.is_array())
                    };
                    if is_array {
                        self.arrays += 1;
                        self.arr_index.insert(key, sidx);
                    }
                    self.order.push(sidx);
                    self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
                    self.live += 1;
                    self.maybe_compact_order();
                    self.ref_of(sidx)
                }
            }
        }
    }

    /// 上值到既有**无值**条目（phantom 占位/启动数组标记）：序位按新
    /// 建处理——移到插入序末尾、旧位留洞。与平表镜像的日志语义逐字
    /// 同（map-insert-new 必进插入日志；无值条目不列——它此前从未在
    /// 列举域出现过）。槽与代数不动（指针稳定不变式）。
    #[allow(dead_code)] // not(embedded) 下 land_value 单探测接管；本形为 embedded 臂
    pub(crate) fn insert_moving(&mut self, key: Rc<str>, var: Var) -> VarRef {
        if let Some(sidx) = self.slot_of(&key) {
            let old_pos = self.slots[sidx as usize].order_pos as usize;
            self.order[old_pos] = NO_SLOT;
            let was_array = self.slots[sidx as usize]
                .var
                .as_ref()
                .is_some_and(|v| v.is_array());
            self.slots[sidx as usize].var = Some(var);
            self.reconcile_arr_index(&key, sidx, was_array);
            self.order.push(sidx);
            self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
            self.maybe_compact_order();
            return self.ref_of(sidx);
        }
        self.insert(key, var)
    }

    /// 快索引与 ARRAY 位对账（位变化点共用）：`was` = 换入前该槽的
    /// ARRAY 位，`now` 取自刚换入的 Var。
    #[inline]
    fn reconcile_arr_index(&mut self, key: &str, sidx: u32, was: bool) {
        let now = self.slots[sidx as usize]
            .var
            .as_ref()
            .is_some_and(|v| v.is_array());
        if now && !was {
            self.arr_index.insert(key.to_string(), sidx);
        } else if !now && was {
            self.arr_index.remove(key);
        }
    }

    /// 值落地的**单探测**形（`global_insert` 咽喉专用）：一次桶链走
    /// 同时产出 was_valued/flags 并按序位语义分派（有值 = 原位覆盖；
    /// 无值 = 移位新建——`insert_moving` 语义；新键 = 追加）。ARRAY
    /// 位逐字保留（计数/索引不随值落地变化）。返回
    /// `(句柄, 是否新建序位)`——后者 ⟺ 平表 map-insert-new（对拍位，
    /// 调用方据此免掉一次先行存在性探测）。键 String 在新键臂被
    /// `Rc::from` 挪用（零拷贝）。
    pub(crate) fn land_value(&mut self, key: String, v: Value) -> (VarRef, bool) {
        #[cfg(not(feature = "embedded"))]
        {
            let bidx = bucket_of(&key, self.buckets.len());
            let mut idx = self.buckets[bidx];
            while idx != NO_SLOT {
                if self.slots[idx as usize].key.as_deref() == Some(key.as_str()) {
                    let sidx = idx;
                    let (was_valued, flags) = {
                        let var = self.slots[sidx as usize].var.as_ref().expect("live slot");
                        (var.has_value(), var.flags)
                    };
                    let mut var = Var::with_value(v);
                    var.flags = flags;
                    if was_valued {
                        self.slots[sidx as usize].var = Some(var);
                        return (self.ref_of(sidx), false);
                    }
                    let old_pos = self.slots[sidx as usize].order_pos as usize;
                    self.order[old_pos] = NO_SLOT;
                    self.slots[sidx as usize].var = Some(var);
                    self.order.push(sidx);
                    self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
                    self.maybe_compact_order();
                    return (self.ref_of(sidx), true);
                }
                idx = self.slots[idx as usize].next;
            }
            let sidx = self.alloc_slot(Rc::from(key), bidx);
            self.slots[sidx as usize].var = Some(Var::with_value(v));
            self.order.push(sidx);
            self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
            self.live += 1;
            self.maybe_grow_buckets();
            self.maybe_compact_order();
            (self.ref_of(sidx), true)
        }
        #[cfg(feature = "embedded")]
        {
            let existing = self.get(&key);
            let (was_valued, flags) = existing
                .map(|x| (x.has_value(), x.flags))
                .unwrap_or((false, VarFlags::default()));
            let mut var = Var::with_value(v);
            var.flags = flags;
            if was_valued {
                let r = self.insert(Rc::from(key.as_str()), var);
                (r, false)
            } else {
                let r = self.insert_moving(Rc::from(key.as_str()), var);
                (r, true)
            }
        }
    }

    /// 有值条目的**单探测**摘除（`global_remove` 咽喉专用）：占位
    /// 条目不动（归 forget_phantom 面）。返回被摘的值。
    pub(crate) fn remove_value(&mut self, key: &str) -> Option<Value> {
        let sidx = self.slot_of(key)?;
        if !self.slots[sidx as usize]
            .var
            .as_ref()
            .expect("live slot")
            .has_value()
        {
            return None;
        }
        #[cfg(not(feature = "embedded"))]
        self.unlink_from_bucket(sidx);
        #[cfg(feature = "embedded")]
        {
            self.index.remove(key);
        }
        let (var, was_array) = {
            let slot = &mut self.slots[sidx as usize];
            let var = slot.var.take().expect("live slot");
            let was = var.is_array();
            slot.key = None;
            slot.gen += 1;
            (var, was)
        };
        if was_array {
            self.arrays -= 1;
            self.arr_index.remove(key);
        }
        self.order[self.slots[sidx as usize].order_pos as usize] = NO_SLOT;
        self.live -= 1;
        self.free.push(sidx);
        self.maybe_compact_order();
        Some(var.value)
    }

    /// 读表项（仅活条目）。
    pub(crate) fn get(&self, key: &str) -> Option<&Var> {
        let i = self.slot_of(key)?;
        Some(self.slots[i as usize].var.as_ref().expect("live slot"))
    }

    /// 写表项（仅活条目）。
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut Var> {
        let i = self.slot_of(key)?;
        Some(self.slots[i as usize].var.as_mut().expect("live slot"))
    }

    /// 删除（置洞 + 代数 +1 + 槽入复用队）。返回被删的表项。
    pub(crate) fn remove(&mut self, key: &str) -> Option<Var> {
        let sidx = self.slot_of(key)?;
        #[cfg(not(feature = "embedded"))]
        self.unlink_from_bucket(sidx);
        #[cfg(feature = "embedded")]
        {
            self.index.remove(key);
        }
        let (var, was_array) = {
            let slot = &mut self.slots[sidx as usize];
            let var = slot.var.take().expect("live slot");
            let was = var.is_array();
            slot.key = None;
            slot.gen += 1;
            (var, was)
        };
        if was_array {
            self.arrays -= 1;
            self.arr_index.remove(key);
        }
        self.order[self.slots[sidx as usize].order_pos as usize] = NO_SLOT;
        self.live -= 1;
        self.free.push(sidx);
        self.maybe_compact_order();
        Some(var)
    }

    /// 「解析一次」：键 → 句柄（无条目 → None）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 的「解析一次」消费面
    pub(crate) fn resolve(&self, key: &str) -> Option<VarRef> {
        self.slot_of(key).map(|i| self.ref_of(i))
    }

    /// 句柄读（代数守卫：不符/空洞 → None）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get_ref(&self, r: &VarRef) -> Option<&Var> {
        let slot = self.slots.get(r.slot as usize)?;
        if slot.gen != r.gen {
            return None;
        }
        slot.var.as_ref()
    }

    /// 句柄写（代数守卫同上）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get_ref_mut(&mut self, r: &VarRef) -> Option<&mut Var> {
        let slot = self.slots.get_mut(r.slot as usize)?;
        if slot.gen != r.gen {
            return None;
        }
        slot.var.as_mut()
    }

    /// 句柄的键（代数守卫同上）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn key_of(&self, r: &VarRef) -> Option<&str> {
        let slot = self.slots.get(r.slot as usize)?;
        if slot.gen != r.gen {
            return None;
        }
        slot.key.as_deref()
    }

    // -- 枚举 ---------------------------------------------------------------

    /// 插入序枚举：跳洞，产出（键, 表项）。「每个活键恰一次，在它最后
    /// 一次创建的位置」——recreated 键的新位即此序（与 var_order 日志
    /// 保留末次出现的重放语义一致）。
    pub(crate) fn order_entries(&self) -> impl Iterator<Item = (&Rc<str>, &Var)> {
        self.order.iter().filter_map(|&sidx| {
            if sidx == NO_SLOT {
                return None;
            }
            let slot = &self.slots[sidx as usize];
            let key = slot.key.as_ref()?;
            let var = slot.var.as_ref()?;
            Some((key, var))
        })
    }

    /// 活表项枚举（顺序不承载语义；introspect 的根快照用）。
    pub(crate) fn values(&self) -> impl Iterator<Item = &Var> {
        self.slots.iter().filter_map(|s| s.var.as_ref())
    }

    /// 前缀匹配的活键（枚举序 = 插入序；purge 面用——调用方自行排序
    /// 或无序消费，与旧 `globals.keys()` 扫描同构）。
    pub(crate) fn keys_with_prefix(&self, prefix: &str) -> Vec<Rc<str>> {
        self.order_entries()
            .filter(|(k, _)| k.as_ref().starts_with(prefix))
            .map(|(k, _)| Rc::clone(k))
            .collect()
    }

    // -- 内部维护 ------------------------------------------------------------

    /// 分配槽（复用空洞或追加；键入槽、桶链接入）。
    #[cfg(not(feature = "embedded"))]
    fn alloc_slot(&mut self, key: Rc<str>, bidx: usize) -> u32 {
        let next = self.buckets[bidx];
        match self.free.pop() {
            Some(sidx) => {
                let slot = &mut self.slots[sidx as usize];
                slot.key = Some(key);
                slot.next = next;
                self.buckets[bidx] = sidx;
                sidx
            }
            None => {
                let sidx = self.slots.len() as u32;
                self.slots.push(Slot {
                    gen: 0,
                    key: Some(key),
                    var: None,
                    next,
                    order_pos: NO_SLOT,
                });
                self.buckets[bidx] = sidx;
                sidx
            }
        }
    }

    #[cfg(feature = "embedded")]
    fn alloc_slot(&mut self, key: Rc<str>) -> u32 {
        match self.free.pop() {
            Some(sidx) => {
                self.index.insert(Rc::clone(&key), sidx);
                self.slots[sidx as usize].key = Some(key);
                sidx
            }
            None => {
                let sidx = self.slots.len() as u32;
                self.index.insert(Rc::clone(&key), sidx);
                self.slots.push(Slot {
                    gen: 0,
                    key: Some(key),
                    var: None,
                    order_pos: NO_SLOT,
                });
                sidx
            }
        }
    }

    /// 桶链摘除（remove 前置）。
    #[cfg(not(feature = "embedded"))]
    fn unlink_from_bucket(&mut self, sidx: u32) {
        let key = self.slots[sidx as usize].key.clone().expect("live slot");
        let bidx = bucket_of(&key, self.buckets.len());
        let mut cur = self.buckets[bidx];
        let mut prev: u32 = NO_SLOT;
        while cur != NO_SLOT {
            if cur == sidx {
                let next = self.slots[cur as usize].next;
                if prev == NO_SLOT {
                    self.buckets[bidx] = next;
                } else {
                    self.slots[prev as usize].next = next;
                }
                self.slots[cur as usize].next = NO_SLOT;
                return;
            }
            prev = cur;
            cur = self.slots[cur as usize].next;
        }
    }

    /// 桶数倍增（负载因子 1 触发）。桶链重挂按槽下标升序——确定性；
    /// 槽不搬移（指针稳定不变式）。
    #[cfg(not(feature = "embedded"))]
    fn maybe_grow_buckets(&mut self) {
        if (self.live as usize) <= self.buckets.len() {
            return;
        }
        let new_len = (self.live as usize).next_power_of_two().max(8);
        self.buckets = vec![NO_SLOT; new_len];
        for i in 0..self.slots.len() {
            let b = match self.slots[i].key.as_deref() {
                Some(k) => bucket_of(k, new_len),
                None => continue,
            };
            let head = self.buckets[b];
            self.slots[i].next = head;
            self.buckets[b] = i as u32;
        }
    }

    /// 插入序向量压缩（洞数超阈值——与 compact_order 同触发形：
    /// `len > 2 * live + 64`）。只重建 `order` 并回填 `order_pos`；
    /// 槽与桶链不动（指针稳定不变式）。
    fn maybe_compact_order(&mut self) {
        if self.order.len() <= 2 * self.live + 64 {
            return;
        }
        let mut order = core::mem::take(&mut self.order);
        let mut pos = 0usize;
        for i in 0..order.len() {
            let sidx = order[i];
            if sidx == NO_SLOT {
                continue;
            }
            self.slots[sidx as usize].order_pos = pos as u32;
            order[pos] = sidx;
            pos += 1;
        }
        order.truncate(pos);
        self.order = order;
    }
}

/// 桶下标（Fx 风格无键哈希，同 varmap.rs 的信任模型：脚本提供的键、
/// 表尺寸小）。枚举序不取自桶链（顺序语义归插入序向量 + tcl_order
/// 重放），故哈希质量只影响探测长度。
#[cfg(not(feature = "embedded"))]
fn bucket_of(key: &str, n_buckets: usize) -> usize {
    let mut h = VarHasher::default();
    h.write(key.as_bytes());
    (h.finish() & (n_buckets as u64 - 1)) as usize
}

// ---------------------------------------------------------------------------
// VarArena
// ---------------------------------------------------------------------------

/// 句柄 arena：表集合 + 按 [`TableId`] 分派的句柄读/写。步骤 1 只装配
/// 根表（`::`，平键域与现 `globals` 同域）；每命名空间一张的拆分与
/// Frame/Array 臂随步骤 2/3 接入。
pub(crate) struct VarArena {
    tables: Vec<VarTable>,
}

impl Default for VarArena {
    fn default() -> Self {
        Self::new()
    }
}

impl VarArena {
    /// 单根表 arena（:: 表在 0 号位）。
    pub(crate) fn new() -> Self {
        Self { tables: vec![VarTable::new()] }
    }

    /// :: 表（步骤 1 的唯一装配表）。
    #[inline]
    pub(crate) fn root(&self) -> &VarTable {
        &self.tables[0]
    }

    #[inline]
    pub(crate) fn root_mut(&mut self) -> &mut VarTable {
        &mut self.tables[0]
    }

    /// 句柄读（代数守卫在表内）。Frame/Array 臂步骤 2/3 接入前恒 None。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get(&self, r: &VarRef) -> Option<&Var> {
        match &r.table {
            TableId::Global(i) => self.tables.get(*i as usize).and_then(|t| t.get_ref(r)),
            _ => None,
        }
    }

    /// 句柄写（代数守卫同上）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get_mut(&mut self, r: &VarRef) -> Option<&mut Var> {
        match &r.table {
            TableId::Global(i) => {
                self.tables.get_mut(*i as usize).and_then(|t| t.get_ref_mut(r))
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(all(test, not(feature = "embedded")))]
mod tests {
    use super::*;

    fn v(s: &str) -> Var {
        Var::with_value(Value::from_str(s))
    }

    #[test]
    fn insert_get_remove_roundtrip() {
        let mut t = VarTable::new();
        let r = t.insert(Rc::from("x"), v("5"));
        assert_eq!(t.get("x").map(|v| v.value.as_str()), Some("5"));
        assert_eq!(t.get_ref(&r).map(|v| v.value.as_str()), Some("5"));
        assert_eq!(t.key_of(&r), Some("x"));
        let removed = t.remove("x").unwrap();
        assert_eq!(removed.value.as_str(), "5");
        assert!(t.get("x").is_none());
        // 代数守卫：旧句柄读取 miss。
        assert!(t.get_ref(&r).is_none());
        assert!(t.key_of(&r).is_none());
    }

    #[test]
    fn slot_reuse_keeps_old_handles_dead() {
        let mut t = VarTable::new();
        let r1 = t.insert(Rc::from("a"), v("1"));
        let _ = t.remove("a");
        // 空洞槽复用：新条目落同一槽位，代数已递增。
        let r2 = t.insert(Rc::from("a"), v("2"));
        assert_eq!(r1.slot, r2.slot);
        assert_ne!(r1.gen, r2.gen);
        assert!(t.get_ref(&r1).is_none());
        assert_eq!(t.get_ref(&r2).map(|v| v.value.as_str()), Some("2"));
        // 覆盖不换句柄：re-set 原地替换。
        let r3 = t.insert(Rc::from("a"), v("3"));
        assert_eq!(r3.slot, r2.slot);
        assert_eq!(r3.gen, r2.gen);
        assert_eq!(t.get_ref(&r2).map(|v| v.value.as_str()), Some("3"));
    }

    #[test]
    fn order_semantics_recreate_takes_new_place() {
        let mut t = VarTable::new();
        for k in ["a", "b", "c"] {
            t.insert(Rc::from(k), v(k));
        }
        fn seq(t: &VarTable) -> Vec<&str> {
            t.order_entries().map(|(k, _)| k.as_ref()).collect()
        }
        assert_eq!(seq(&t), vec!["a", "b", "c"]);
        // 纯覆盖保持原位。
        t.insert(Rc::from("b"), v("B"));
        assert_eq!(seq(&t), vec!["a", "b", "c"]);
        // 删除 + 重建取新位（与 var_order 日志保留末次出现一致）。
        assert!(t.remove("b").is_some());
        assert_eq!(seq(&t), vec!["a", "c"]);
        t.insert(Rc::from("b"), v("b2"));
        assert_eq!(seq(&t), vec!["a", "c", "b"]);
    }

    #[test]
    fn order_compaction_preserves_sequence() {
        let mut t = VarTable::new();
        // 反复建删逼出 order 压缩（len > 2*live + 64）。
        for i in 0..64u32 {
            let k = Rc::from(format!("k{i}").as_str());
            t.insert(k, v("x"));
            t.remove(format!("k{i}").as_str());
        }
        t.insert(Rc::from("live"), v("L"));
        let seq: Vec<&str> = t.order_entries().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(seq, vec!["live"]);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn flags_and_array_count() {
        let mut t = VarTable::new();
        let mut av = v("arr");
        av.flags.insert(VarFlags::ARRAY);
        av.elem_count = 3;
        t.insert(Rc::from("arr"), av);
        t.insert(Rc::from("s"), v("1"));
        assert_eq!(t.array_count(), 1);
        assert!(t.get("arr").unwrap().is_array());
        assert_eq!(t.get("arr").unwrap().elem_count, 3);
        let mut uv = v("p");
        uv.flags.insert(VarFlags::UNDEFINED);
        t.insert(Rc::from("p"), uv);
        assert!(t.get("p").unwrap().is_undefined());
        // 覆盖撤 ARRAY 位 → 计数回落。
        t.insert(Rc::from("arr"), v("scalar now"));
        assert_eq!(t.array_count(), 0);
        assert!(t.remove("arr").is_some());
        assert_eq!(t.array_count(), 0);
    }

    #[test]
    fn keys_with_prefix_and_values() {
        let mut t = VarTable::new();
        for k in ["n::a", "n::b(1)", "solo", "n::b(2)"] {
            t.insert(Rc::from(k), v("x"));
        }
        let pre = t.keys_with_prefix("n::");
        assert_eq!(
            pre.iter().map(|k| k.as_ref()).collect::<Vec<_>>(),
            vec!["n::a", "n::b(1)", "n::b(2)"]
        );
        assert_eq!(t.values().count(), 4);
        assert!(t.resolve("n::a").is_some_and(|r| t.get_ref(&r).is_some()));
        assert!(t.resolve("missing").is_none());
    }

    #[test]
    fn arena_handle_dispatch() {
        let mut a = VarArena::new();
        let r = a.root_mut().insert(Rc::from("g"), v("9"));
        assert_eq!(a.get(&r).map(|v| v.value.as_str()), Some("9"));
        a.get_mut(&r).unwrap().value = Value::from_int(10);
        assert_eq!(a.root().get("g").map(|v| v.value.as_str()), Some("10"));
        // Frame 臂（步骤 2 前占位）：恒 None。
        let fr = VarRef { table: TableId::Frame(0), slot: 0, gen: 0 };
        assert!(a.get(&fr).is_none());
        assert!(a.get_mut(&fr).is_none());
    }

    #[test]
    fn insert_moving_matches_log_semantics() {
        let mut t = VarTable::new();
        // 无值占位（启动标记形）先入表、序位领先于后续插入。
        let mut marker = Var::with_value(Value::unset());
        marker.flags.insert(VarFlags::ARRAY);
        t.insert(Rc::from("env"), marker);
        t.insert(Rc::from("a"), v("A"));
        t.insert(Rc::from("b"), v("B"));
        fn seq(t: &VarTable) -> Vec<&str> {
            t.order_entries()
                .filter(|(_, v)| v.has_value())
                .map(|(k, _)| k.as_ref())
                .collect()
        }
        // 占位无值不列。
        assert_eq!(seq(&t), vec!["a", "b"]);
        // 上值 = 新建序位（镜像 map-insert-new 必进日志——env 落到末
        // 尾，与平表重放逐字同）。
        let mut ev = v("ENV");
        ev.flags.insert(VarFlags::ARRAY);
        t.insert_moving(Rc::from("env"), ev);
        assert_eq!(seq(&t), vec!["a", "b", "env"]);
        // 有值覆盖 = 原位（insert 不动序）。
        t.insert(Rc::from("a"), v("A2"));
        assert_eq!(seq(&t), vec!["a", "b", "env"]);
        assert_eq!(t.get("a").map(|x| x.value.as_str()), Some("A2"));
        assert!(t.get("env").unwrap().is_array());
    }

    #[test]
    fn bucket_growth_no_rehash_loss() {
        let mut t = VarTable::new();
        // 桶倍增跨负载因子多次：全键可读、顺序完整。
        let n = 500u32;
        for i in 0..n {
            t.insert(Rc::from(format!("k{i}").as_str()), v("x"));
        }
        assert_eq!(t.len(), n as usize);
        for i in 0..n {
            assert!(t.get(&format!("k{i}")).is_some(), "k{i}");
        }
        let keys: Vec<&str> = t.order_entries().map(|(k, _)| k.as_ref()).collect();
        let want: Vec<String> = (0..n).map(|i| format!("k{i}")).collect();
        assert_eq!(keys, want.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    }
}
