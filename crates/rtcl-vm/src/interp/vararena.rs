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
//! 表实例身份：每张 [`VarTable`] 携带自己的 [`TableId`]——0 号根表
//! （`::`，平键域与现 `globals` 同域）为 [`TableId::Global`]；步骤 2
//! 的帧表驻 [`super::CallFrame`]（池化归还时 [`VarTable::reset`] 清空
//! + 代数递增；协程排水随帧整体进出 record），实例携带
//! [`TableId::Frame`]（帧栈下标，推栈时刷新）——句柄的读/写对帧表经
//! 帧自身（arena 不持有帧表；[`VarArena::get`] 的 Frame 臂保持 None
//! 占位）。

#[cfg(not(feature = "embedded"))]
use super::varmap::VarHasher;
#[cfg(not(feature = "embedded"))]
use std::hash::Hasher as _;
#[cfg(not(feature = "embedded"))]
use std::rc::Rc;
#[cfg(feature = "embedded")]
use alloc::boxed::Box;
#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HistSet;
#[cfg(feature = "embedded")]
use alloc::rc::Rc;
#[cfg(feature = "embedded")]
use alloc::vec;
#[cfg(feature = "embedded")]
use alloc::string::String;
#[cfg(feature = "embedded")]
use alloc::vec::Vec;
#[cfg(not(feature = "embedded"))]
use std::collections::HashSet as HistSet;
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

/// 表身份（D1：三类表）。`Global(0)` = arena 根表；`Frame(i)` = 帧表
/// （步骤 2：表实例驻 [`super::CallFrame`]，i = 帧栈下标）；`Array`
/// = 数组元素表（步骤 3：宿主基 Var 的子表，按宿主 [`VarRef`] 寻址
/// ——宿主在全局/帧表里，子表实例驻宿主 [`Var::elements`]）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TableId {
    /// 全局（命名空间）表：[`VarArena::tables`] 的下标。
    Global(u32),
    /// 帧变量表（步骤 2 迁移；表实例驻帧，句柄经帧侧访问）。
    Frame(u32),
    /// 数组元素表（步骤 3）——基变量的句柄（寻址经宿主 Var 的
    /// `elements` 子表；生命周期随宿主：宿主摘除/帧池重置即回收）。
    Array(Box<VarRef>),
}

impl TableId {
    /// 同表判定（句柄读的代数守卫前置——张冠李戴的句柄一律 miss）。
    #[inline]
    pub(crate) fn matches(&self, other: &TableId) -> bool {
        match (self, other) {
            (TableId::Global(a), TableId::Global(b)) => a == b,
            (TableId::Frame(a), TableId::Frame(b)) => a == b,
            (TableId::Array(a), TableId::Array(b)) => a == b,
            _ => false,
        }
    }
}

/// 「解析一次」返回的句柄：表 + 槽 + 代数。槽删除后代数递增，持旧
/// 句柄的读取一律 miss（返回 None）——指针稳定与 ABA 防护并存。
/// （步骤 3 起 `Array` 身份的句柄寻址宿主 Var 的子表。）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// 数组容器。全局平键域的数组基标记（镜像 `array_globals`）；
    /// 帧域随步骤 2 迁入（帧表的数组基同位——帧 `array_locals` 的
    /// debug 镜像；帧元素计数仍归帧 `elem_counts`，步骤 3 并入）。
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

/// 变量 trace 集（D1 步骤 4，specs/ARCH-DESIGN-20261126.md D1）：
/// `trace add variable` 的挂靠点 = Var 上的直接字段（[`Var::traces`]，
/// `Option<Box<TraceSet>>`——只有被追踪的 Var 付堆侧成本）。两个族：
///
/// - **整变量族**（`var`）：`trace add variable <name>`（标量/数组基
///   同域）——注册序存放，触发时组内 latest-first。
/// - **元素族**（`elem`）：`trace add variable <base>(<idx>)`——挂
///   **宿主（数组基）Var**，元素名 → 该元素名下的 trace（注册序）。
///   挂宿主而非元素 Var 是生命周期语义：元素摘除（unset/整数组扫除）
///   后 trace 仍在（再 set 仍触发——基线钉形）、phantom 元素（trace
///   注册在从未落值的元素上，子表无条目）同域存放——与旧
///   `elem_traces`（stamp 键 → 元素名 → traces）的触发生命周期逐字
///   同，键字符串退役。
///
/// 本结构只承载数据；收集/触发的语义层在 `vars.rs`（fire 核心）。
#[derive(Debug, Clone, Default)]
pub(crate) struct TraceSet {
    /// 整变量族（注册序）。
    pub var: Vec<super::VarTrace>,
    /// 整变量族「键在册」位：首次整变量注册置位，此后**不随记录摘除
    /// 复位**（旧 var_traces 字符串键的 lingering 语义——摘净后
    /// upvar-8.7 的 contains_key 判定仍拦，基线钉形 probe8c）；宿主
    /// 整体消亡（快照 take）随之消失。元素族注册不置位（upvar-8.7
    /// 只看整变量族——元素 trace 走 exists 检查，oracle upvar_levels
    /// u24 钉形）。
    pub var_linger: bool,
    /// 元素族：元素名 → 该元素名下的 trace（注册序）。小集合线性扫
    /// （被追踪的元素数以个位数计；顺序确定、无哈希器依赖——嵌入
    /// 构建同形）。
    pub elem: Vec<(Rc<str>, Vec<super::VarTrace>)>,
}

impl TraceSet {
    /// 无任何注册记录。
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.var.is_empty() && self.elem.is_empty()
    }

    /// 注册记录总数（整变量 + 全部元素族）——快照火的调用侧对账用
    /// （快径守卫真源是 `Interp::n_trace_hosts` 的宿主在席计数）。
    #[inline]
    pub(crate) fn record_count(&self) -> usize {
        self.var.len() + self.elem.iter().map(|(_, v)| v.len()).sum::<usize>()
    }

    /// 整变量族追加（注册；键在册位随之置位——lingering）。
    pub(crate) fn push_var(&mut self, t: super::VarTrace) {
        self.var.push(t);
        self.var_linger = true;
    }

    /// 元素族追加（注册）。返回该元素的 vec（无则新建尾条目）。
    pub(crate) fn elem_entry_mut(&mut self, elem: &str) -> &mut Vec<super::VarTrace> {
        if let Some(pos) = self.elem.iter().position(|(k, _)| k.as_ref() == elem) {
            let last = &mut self.elem[pos];
            return &mut last.1;
        }
        self.elem.push((Rc::from(elem), Vec::new()));
        let pos = self.elem.len() - 1;
        let last = &mut self.elem[pos];
        &mut last.1
    }

    /// 整变量族摘除第一条精确匹配（ops+script——remove 逐条删语义）。
    /// 返回是否摘到。
    pub(crate) fn remove_first_var(&mut self, ops: &[String], script: &str) -> bool {
        if let Some(i) = self.var.iter().position(|t| t.ops == ops && t.script == script) {
            self.var.remove(i);
            true
        } else {
            false
        }
    }

    /// 元素族摘除指定元素的第一条精确匹配；vec 摘空后条目一并退场。
    /// 返回是否摘到。
    pub(crate) fn remove_first_elem(
        &mut self,
        elem: &str,
        ops: &[String],
        script: &str,
    ) -> bool {
        let Some(pos) = self.elem.iter().position(|(k, _)| k.as_ref() == elem) else {
            return false;
        };
        let v = &mut self.elem[pos].1;
        let Some(i) = v.iter().position(|t| t.ops == ops && t.script == script) else {
            return false;
        };
        v.remove(i);
        if v.is_empty() {
            self.elem.remove(pos);
        }
        true
    }

    /// 元素族的整条摘除（缺失元素 unset 抹记录——trace-14.17 族）。
    /// 返回摘掉的记录数。
    pub(crate) fn remove_elem_entry(&mut self, elem: &str) -> usize {
        let Some(pos) = self.elem.iter().position(|(k, _)| k.as_ref() == elem) else {
            return 0;
        };
        let n = self.elem[pos].1.len();
        self.elem.remove(pos);
        n
    }

    /// 元素族条目读（触发收集面）。
    #[inline]
    pub(crate) fn elem_vec(&self, elem: &str) -> Option<&[super::VarTrace]> {
        self.elem
            .iter()
            .find(|(k, _)| k.as_ref() == elem)
            .map(|(_, v)| v.as_slice())
    }

    /// 触发收集（`op` 下的脚本快照）：（整变量组, 元素组）——各按注册
    /// （最旧在前）序产出；触发序（latest-first、整组先于元素组 A1E1）
    /// 由 vars.rs 的火循环统一施加。
    pub(crate) fn firing_scripts(
        &self,
        elem: Option<&str>,
        op: &str,
    ) -> (Vec<String>, Vec<String>) {
        let whole: Vec<String> = self
            .var
            .iter()
            .filter(|t| t.ops.iter().any(|o| o == op))
            .map(|t| t.script.clone())
            .collect();
        let elems: Vec<String> = match elem {
            Some(e) => self
                .elem_vec(e)
                .map(|trs| {
                    trs.iter()
                        .filter(|t| t.ops.iter().any(|o| o == op))
                        .map(|t| t.script.clone())
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };
        (whole, elems)
    }
}

/// 链接目标占位（步骤 3/4：链接/死链面迁入；本批恒 `None`）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum LinkTarget {}

/// 表项（D1 目标形状）：值 + 形态位 + 存活元素数 + 元素子表 +
/// trace/链接占位。
#[derive(Debug, Clone)]
pub(crate) struct Var {
    /// 值。无值条目为 [`Value::unset`] 哨兵——「有值 ⟺ 旧路径
    /// `globals` 含键」独立于 [`VarFlags::UNDEFINED`]（后者只镜像
    /// phantom 成员位）。
    pub value: Value,
    pub flags: VarFlags,
    /// ARRAY 时：存活元素数（T7 `elem_counts` 的 Var 内化——全局域步
    /// 骤 1、帧域步骤 3 并入；镜像 `elem_counts` 退 debug 对拍域）。
    pub elem_count: u32,
    /// ARRAY 时：本数组的元素表（步骤 3——D1 的第三类表，按宿主
    /// [`VarRef`] 寻址：子表实例携带 `TableId::Array(宿主句柄)` 身份，
    /// 句柄读写经 [`VarArena::get`] 的 Array 臂或宿主所在表的
    /// [`VarTable::get_ref`]。生命周期随宿主 Var：数组 unset（宿主摘
    /// 除）、帧池重置（[`VarTable::reset`] 清槽）、 ARRAY 位撤销时
    /// 整表回收）。不变式：`is_array() ⟺ elements.is_some()`——
    /// [`VarTable`] 的写槽路径统一维护（见 `sync_elements`）。
    pub elements: Option<Box<VarTable>>,
    /// 变量/元素 trace 集（D1 步骤 4 挂载——`Box` 嵌套保持 `Var` 的
    /// 尺寸增量为一个指针宽：只有被追踪的 Var 付堆侧 TraceSet 成本）。
    pub traces: Option<Box<TraceSet>>,
    /// 链接目标（步骤 5 链接面迁入；本批恒 `None`）。
    #[allow(dead_code)]
    pub link: Option<LinkTarget>,
}

impl Var {
    /// 本 Var 的 trace 集（无则 `None`）。
    #[inline]
    pub(crate) fn trace_set(&self) -> Option<&TraceSet> {
        self.traces.as_deref()
    }

    /// trace 集就地确保存在（注册位点）。返回是否**新建**了集合——
    /// `Interp` 层据此刻 n_trace_hosts 计数（宿主在席 = Box 在席，与旧
    /// 字符串表「键一旦建立即 linger」的守卫语义逐字同）。
    pub(crate) fn traces_ensure(&mut self) -> (bool, &mut TraceSet) {
        if self.traces.is_none() {
            self.traces = Some(Box::new(TraceSet::default()));
            (true, self.traces.as_mut().unwrap())
        } else {
            (false, self.traces.as_mut().unwrap())
        }
    }
}

impl Var {
    /// 带值的表项（其余位为默认）。
    pub(crate) fn with_value(v: Value) -> Self {
        Self {
            value: v,
            flags: VarFlags::default(),
            elem_count: 0,
            elements: None,
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

    /// 元素子表（ARRAY 时 Some）。
    #[inline]
    pub(crate) fn elements(&self) -> Option<&VarTable> {
        self.elements.as_deref()
    }

    #[inline]
    pub(crate) fn elements_mut(&mut self) -> Option<&mut VarTable> {
        self.elements.as_deref_mut()
    }
}

// ---------------------------------------------------------------------------
// 槽
// ---------------------------------------------------------------------------

/// arena 槽：删除只置洞 + 代数 +1——槽地址永不搬移，`&Var` 指针稳定；
/// 空洞槽位可复用（代数防护旧句柄）。
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub(crate) struct VarTable {
    /// 表身份（句柄产出与句柄读的归属校验）。根表 = Global(0)；帧表
    /// = Frame(帧栈下标，推栈时刷新——池化帧可换深度复用）。
    owner: TableId,
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
    /// **元素表专属**：曾经插入过的键（下标）序——`array statistics`
    /// 的桶轨迹重放（tclsh 的元素表保留其长成的桶）。语义 = 「自
    /// （重）建以来插入过的每个键恰一次，在它最后一次创建的位置」，
    /// 与共享插入日志的历史面（旧 `array_elements(live_only=false)`
    /// 的重放域）同形；压缩按标准触发形（`len > 2*live + 64`）保留
    /// 末次出现、活性过滤（删除键落档——桶数由此回落，与旧共享日志
    /// 压缩后的统计面同形）。非元素表恒空。
    history: Vec<Rc<str>>,
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
    /// 空表（根表身份：Global(0)）。
    pub(crate) fn new() -> Self {
        Self::with_owner(TableId::Global(0))
    }

    /// 帧表（Frame(帧栈下标) 身份）。
    #[allow(dead_code)] // 步骤 2 帧表构造（call.rs）
    pub(crate) fn new_frame(frame_idx: u32) -> Self {
        Self::with_owner(TableId::Frame(frame_idx))
    }

    /// 数组元素表（步骤 3：`TableId::Array(宿主句柄)` 身份——宿主 Var
    /// 持有的子表，宿主摘除即随之回收）。
    #[allow(dead_code)] // 步骤 3 元素表构造（sync_elements）
    pub(crate) fn new_array(host: VarRef) -> Self {
        Self::with_owner(TableId::Array(Box::new(host)))
    }

    /// 指定身份的空表。
    pub(crate) fn with_owner(owner: TableId) -> Self {
        Self {
            owner,
            slots: Vec::new(),
            #[cfg(not(feature = "embedded"))]
            buckets: vec![NO_SLOT; 8],
            #[cfg(feature = "embedded")]
            index: alloc::collections::BTreeMap::new(),
            order: Vec::new(),
            live: 0,
            free: Vec::new(),
            history: Vec::new(),
            arrays: 0,
            #[cfg(not(feature = "embedded"))]
            arr_index: super::varmap::VarMap::default(),
            #[cfg(feature = "embedded")]
            arr_index: alloc::collections::BTreeMap::new(),
        }
    }

    /// 刷新表身份（池化帧换深度复用时——推栈点调用）。
    #[allow(dead_code)] // 步骤 2 推栈点（call.rs / coro.rs）
    pub(crate) fn set_owner(&mut self, owner: TableId) {
        self.owner = owner;
    }

    /// 整表清空（**池化重置语义**——帧归还/尾调重绑/trace 清场共用的
    /// 「表清空」）：全部条目摘除、代数逐槽 +1（旧句柄即使槽号被复用
    /// 也因代数不符 miss——「代数不冲突」）、桶链/插入序/数组基索引/
    /// 复用队复位；槽与桶的分配容量保留（池化帧的表重置成本 = 逐槽
    /// gen 自增 + 桶清零 + 复用队重填，均为 O(槽数) 无重分配）。
    pub(crate) fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.gen = slot.gen.wrapping_add(1);
            slot.key = None;
            slot.var = None;
            #[cfg(not(feature = "embedded"))]
            {
                slot.next = NO_SLOT;
            }
            slot.order_pos = NO_SLOT;
        }
        #[cfg(not(feature = "embedded"))]
        {
            self.buckets.fill(NO_SLOT);
        }
        #[cfg(feature = "embedded")]
        {
            self.index.clear();
        }
        self.order.clear();
        self.live = 0;
        self.free.clear();
        self.history.clear();
        // 复用队重填（栈序：0 号槽最先复用——局部性友好；插入序语义
        // 不依赖槽号，只依赖 order 向量）。
        self.free.extend((0..self.slots.len() as u32).rev());
        self.arrays = 0;
        self.arr_index.clear();
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

    /// 数组基标记（位 + 计数；快索引对账；子表随 ARRAY 位建立——
    /// `sync_elements`）。无条目时建无值 Var（启动窗口：标记先于基
    /// 键落表——插入序红线不动）。已标记时只更新计数。
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
                    self.sync_elements(sidx);
                }
            }
            None => {
                let mut var = Var::with_value(Value::unset());
                var.flags.insert(VarFlags::ARRAY);
                if let Some(n) = count {
                    var.elem_count = n;
                }
                // 走 insert（ARRAY 条目入表时快索引自动对账 + 子表建立）。
                self.insert(Rc::from(key), var);
            }
        }
    }

    /// 数组基摘标（计数随位同清；快索引对账；子表随 ARRAY 位撤销整
    /// 表回收）。
    pub(crate) fn unmark_array(&mut self, key: &str) {
        if let Some(&sidx) = self.arr_index.get(key) {
            let slot = &mut self.slots[sidx as usize];
            let var = slot.var.as_mut().expect("indexed slot");
            var.flags.remove(VarFlags::ARRAY);
            var.elem_count = 0;
            var.elements = None;
            self.arrays -= 1;
            self.arr_index.remove(key);
        }
    }

    /// 是否持有该键的条目（有值或占位皆算——「表里有这条 Var」）。
    #[allow(dead_code)] // 表 API 面（测试用；步骤 2/3 接线）
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.slot_of(key).is_some()
    }

    // -- 元素子表同步（步骤 3）------------------------------------------
    //
    // 不变式：`is_array() ⟺ elements.is_some()`。所有「槽内 Var 整体
    // 换入」的路径（insert/land_value/insert_moving）换入后统一过
    // [`Self::sync_elements`]：ARRAY 条目补建子表（携带宿主句柄身份）、
    // 换入前先从旧 Var **接管**子表（纯覆盖不得丢元素——元素读写走子
    // 表后，宿主覆盖只发生在数组基键的空值落地，接管即保真）；非
    // ARRAY 条目子表归 None（回收）。

    /// 槽内 Var 的子表/ARRAY 位同步（写槽后调用）。宿主句柄只在补建
    /// 臂构造（元素/标量写热径零分配——TableId::Array 的 Box 克隆不
    /// 落在每次写上）。
    fn sync_elements(&mut self, sidx: u32) {
        let is_array = self.slots[sidx as usize]
            .var
            .as_ref()
            .expect("live slot")
            .is_array();
        if !is_array {
            self.slots[sidx as usize].var.as_mut().expect("live slot").elements = None;
            return;
        }
        let has_sub = self.slots[sidx as usize]
            .var
            .as_ref()
            .expect("live slot")
            .elements
            .is_some();
        if !has_sub {
            let host = self.ref_of(sidx);
            let var = self.slots[sidx as usize].var.as_mut().expect("live slot");
            var.elements = Some(Box::new(VarTable::new_array(host)));
        }
    }

    /// 元素条目落地（子表咽喉——元素写位点共用）：`&str` 下标直落
    /// （键 Rc 只在新键臂分配——覆盖写零分配），新键入档 history
    /// （推槽键 Rc 的引计克隆——零新增分配）并按触发形压缩。返回是
    /// 否新建。
    pub(crate) fn land_element(&mut self, index: &str, v: Value) -> bool {
        if let Some(sidx) = self.land_value_str(index, v) {
            // history 推槽键 Rc 的引计克隆（零新增分配——键已在槽上）。
            if let Some(k) = self.slots[sidx as usize].key.clone() {
                self.history.push(k);
            }
            if self.history.len() > 2 * self.live + 64 {
                // 活性集先行（不可变阶段——history 的借用随块收口），
                // retain 前放借用。
                let keep: Vec<bool> = {
                    let mut seen: HistSet<&str> = HistSet::new();
                    let mut keep = vec![false; self.history.len()];
                    for (i, k) in self.history.iter().enumerate().rev() {
                        if self.contains(k) && seen.insert(k.as_ref()) {
                            keep[i] = true;
                        }
                    }
                    keep
                };
                let mut i = 0;
                self.history.retain(|_| {
                    let k = keep[i];
                    i += 1;
                    k
                });
            }
            return true;
        }
        false
    }

    /// 元素键入档（元素表专属；新键才推——调用方在元素新建位点调
    /// 用），超限压缩（活性过滤 + 末次出现保留——共享日志压缩同形，
    /// 桶轨迹统计面的边界行为与旧日志重放一致）。
    #[allow(dead_code)] // land_element 内联了入档；独立形留给测试
    pub(crate) fn history_push(&mut self, index: &str) {
        self.history.push(Rc::from(index));
        self.history_compact();
    }

    /// history 压缩（活性过滤 + 末次出现保留——触发形 `len > 2*live
    /// + 64`；桶轨迹统计面的边界行为与旧日志重放一致）。
    fn history_compact(&mut self) {
        if self.history.len() > 2 * self.live + 64 {
            // 活性集先行（不可变阶段——history 的借用随块收口），
            // retain 前放借用。
            let keep: Vec<bool> = {
                let mut seen: HistSet<&str> = HistSet::new();
                let mut keep = vec![false; self.history.len()];
                for (i, k) in self.history.iter().enumerate().rev() {
                    if self.contains(k) && seen.insert(k.as_ref()) {
                        keep[i] = true;
                    }
                }
                keep
            };
            let mut i = 0;
            self.history.retain(|_| {
                let k = keep[i];
                i += 1;
                k
            });
        }
    }

    /// 历史键枚举（`array statistics` 的桶轨迹重放域——每个曾插入键
    /// 恰一次，末次出现序；压缩后的落档语义见 [`Self::history_push`]）。
    #[allow(dead_code)] // 步骤 3 路由接线（array statistics）
    pub(crate) fn history_keys(&self) -> impl Iterator<Item = &str> {
        self.history.iter().map(|k| k.as_ref())
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

    /// 槽下标 → VarRef（表身份 + 代数入柄）。
    fn ref_of(&self, idx: u32) -> VarRef {
        VarRef {
            table: self.owner.clone(),
            slot: idx,
            gen: self.slots[idx as usize].gen,
        }
    }

    /// 句柄归属校验（张冠李戴的句柄——别的表的槽号——一律 miss）。
    #[inline]
    fn owns(&self, r: &VarRef) -> bool {
        self.owner.matches(&r.table)
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
                        // 步骤 3：接管旧 Var 的子表（纯覆盖不丢元素——
                        // 元素读写已迁子表，宿主覆盖只发生在基键空值
                        // 落地等少数路径；换入 Var 自带子表时以换入为
                        // 准）。步骤 4：trace 集同法接管（值落地不得
                        // 断 trace——phantom/占位宿主上值的主径）。
                        let mut var = var;
                        if var.elements.is_none() {
                            if let Some(old) = slot.var.as_mut() {
                                var.elements = old.elements.take();
                            }
                        }
                        if var.traces.is_none() {
                            if let Some(old) = slot.var.as_mut() {
                                var.traces = old.traces.take();
                            }
                        }
                        slot.var = Some(var);
                        was
                    };
                    if was_array != new_is_array {
                        self.arrays += u32::from(new_is_array);
                        self.arrays -= u32::from(was_array);
                        self.reconcile_arr_index(&key, idx, was_array);
                    }
                    self.sync_elements(idx);
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
            self.sync_elements(sidx);
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
                        let mut var = var;
                        if var.elements.is_none() {
                            if let Some(old) = slot.var.as_mut() {
                                var.elements = old.elements.take();
                            }
                        }
                        if var.traces.is_none() {
                            if let Some(old) = slot.var.as_mut() {
                                var.traces = old.traces.take();
                            }
                        }
                        slot.var = Some(var);
                        was
                    };
                    if was_array != new_is_array {
                        self.arrays += u32::from(new_is_array);
                        self.arrays -= u32::from(was_array);
                        self.reconcile_arr_index(&key, sidx, was_array);
                    }
                    self.sync_elements(sidx);
                    self.ref_of(sidx)
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
                    self.sync_elements(sidx);
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
            {
                let slot = &mut self.slots[sidx as usize];
                let mut var = var;
                if var.elements.is_none() {
                    if let Some(old) = slot.var.as_mut() {
                        var.elements = old.elements.take();
                    }
                }
                if var.traces.is_none() {
                    if let Some(old) = slot.var.as_mut() {
                        var.traces = old.traces.take();
                    }
                }
                slot.var = Some(var);
            }
            self.reconcile_arr_index(&key, sidx, was_array);
            self.sync_elements(sidx);
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
                    let (was_valued, flags, elem_count) = {
                        let var = self.slots[sidx as usize].var.as_ref().expect("live slot");
                        (var.has_value(), var.flags, var.elem_count)
                    };
                    let mut var = Var::with_value(v);
                    var.flags = flags;
                    // 步骤 3：元素计数与子表随条目接管（ARRAY 基键的
                    // 值落地不得清账——计数/子表挂宿主 Var）。步骤 4：
                    // trace 集同法接管（值落地不得断 trace）。
                    var.elem_count = elem_count;
                    let old_elements = self.slots[sidx as usize]
                        .var
                        .as_mut()
                        .expect("live slot")
                        .elements
                        .take();
                    if var.elements.is_none() {
                        var.elements = old_elements;
                    }
                    var.traces = self.slots[sidx as usize]
                        .var
                        .as_mut()
                        .expect("live slot")
                        .traces
                        .take();
                    if was_valued {
                        self.slots[sidx as usize].var = Some(var);
                        self.sync_elements(sidx);
                        return (self.ref_of(sidx), false);
                    }
                    let old_pos = self.slots[sidx as usize].order_pos as usize;
                    self.order[old_pos] = NO_SLOT;
                    self.slots[sidx as usize].var = Some(var);
                    self.sync_elements(sidx);
                    self.order.push(sidx);
                    self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
                    self.maybe_compact_order();
                    return (self.ref_of(sidx), true);
                }
                idx = self.slots[idx as usize].next;
            }
            let sidx = self.alloc_slot(Rc::from(key), bidx);
            self.slots[sidx as usize].var = Some(Var::with_value(v));
            self.sync_elements(sidx);
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

    /// [`Self::land_value`] 的 `&str` 形（元素写热径）：键 Rc 只在新
    /// 键臂分配——覆盖写零分配（与 String 形同序位语义）。返回
    /// `Some(槽号)` = 新建（新键/占位上值的移位新建），`None` = 有值
    /// 覆盖。embedded 后备退 get+insert 双探测。
    pub(crate) fn land_value_str(&mut self, key: &str, v: Value) -> Option<u32> {
        #[cfg(not(feature = "embedded"))]
        {
            let bidx = bucket_of(key, self.buckets.len());
            let mut idx = self.buckets[bidx];
            while idx != NO_SLOT {
                if self.slots[idx as usize].key.as_deref() == Some(key) {
                    let sidx = idx;
                    let (was_valued, flags, elem_count) = {
                        let var = self.slots[sidx as usize].var.as_ref().expect("live slot");
                        (var.has_value(), var.flags, var.elem_count)
                    };
                    let mut var = Var::with_value(v);
                    var.flags = flags;
                    var.elem_count = elem_count;
                    let old_elements = self.slots[sidx as usize]
                        .var
                        .as_mut()
                        .expect("live slot")
                        .elements
                        .take();
                    if var.elements.is_none() {
                        var.elements = old_elements;
                    }
                    var.traces = self.slots[sidx as usize]
                        .var
                        .as_mut()
                        .expect("live slot")
                        .traces
                        .take();
                    if was_valued {
                        self.slots[sidx as usize].var = Some(var);
                        self.sync_elements(sidx);
                        return None;
                    }
                    let old_pos = self.slots[sidx as usize].order_pos as usize;
                    self.order[old_pos] = NO_SLOT;
                    self.slots[sidx as usize].var = Some(var);
                    self.sync_elements(sidx);
                    self.order.push(sidx);
                    self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
                    self.maybe_compact_order();
                    return Some(sidx);
                }
                idx = self.slots[idx as usize].next;
            }
            let sidx = self.alloc_slot(Rc::from(key), bidx);
            self.slots[sidx as usize].var = Some(Var::with_value(v));
            self.sync_elements(sidx);
            self.order.push(sidx);
            self.slots[sidx as usize].order_pos = (self.order.len() - 1) as u32;
            self.live += 1;
            self.maybe_grow_buckets();
            self.maybe_compact_order();
            Some(sidx)
        }
        #[cfg(feature = "embedded")]
        {
            let existing = self.get(key);
            let was_valued = existing.is_some_and(|x| x.has_value());
            if was_valued {
                self.insert(Rc::from(key), Var::with_value(v));
                None
            } else {
                let r = self.insert_moving(Rc::from(key), Var::with_value(v));
                Some(r.slot)
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

    /// 就地确保条目（无则建无值占位 Var）——D1 步骤 4 的 trace 注册宿
    /// 主（帧域）。占位条目按无值域过滤（列举/在席探针不可见）；插入
    /// 序向量的占位条目位置随建随定（上值走 insert_moving 移位——与
    /// 日志 map-insert-new 语义逐字同）。
    pub(crate) fn entry_ensure(&mut self, key: &str) -> &mut Var {
        if self.slot_of(key).is_none() {
            self.insert(Rc::from(key), Var::with_value(Value::unset()));
        }
        self.get_mut(key).expect("ensured entry")
    }

    /// 句柄读（表归属 + 代数守卫：不符/空洞 → None）。步骤 3：`Array`
    /// 身份的句柄寻址宿主 Var 的元素子表——宿主在本表时经子表下钻
    /// （子表的归属/代数守卫照常；`TableId::matches` 的 Array 同形
    /// 判定 + 宿主句柄校验双重防张冠李戴）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get_ref(&self, r: &VarRef) -> Option<&Var> {
        if self.owns(r) {
            let slot = self.slots.get(r.slot as usize)?;
            if slot.gen != r.gen {
                return None;
            }
            return slot.var.as_ref();
        }
        if let TableId::Array(host) = &r.table {
            if self.owns(host) {
                return self
                    .get_ref(host)?
                    .elements()
                    .and_then(|t| t.get_ref(r));
            }
        }
        None
    }

    /// 句柄写（守卫同上；Array 句柄经宿主子表下钻）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn get_ref_mut(&mut self, r: &VarRef) -> Option<&mut Var> {
        if self.owns(r) {
            let slot = self.slots.get_mut(r.slot as usize)?;
            if slot.gen != r.gen {
                return None;
            }
            return slot.var.as_mut();
        }
        if let TableId::Array(host) = &r.table {
            let host = host.as_ref().clone();
            if self.owns(&host) {
                return self
                    .get_ref_mut(&host)?
                    .elements
                    .as_deref_mut()
                    .and_then(|t| t.get_ref_mut(r));
            }
        }
        None
    }

    /// 句柄的键（守卫同上）。
    #[allow(dead_code)] // 句柄 API——步骤 2/3 接线
    pub(crate) fn key_of(&self, r: &VarRef) -> Option<&str> {
        if !self.owns(r) {
            return None;
        }
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

/// 句柄 arena：**全局表**集合 + 按 [`TableId`] 分派的句柄读/写。步骤 1
/// 装配根表（`::`，平键域与现 `globals` 同域）；每命名空间一张的拆分
/// 随后续步骤接入。步骤 2 的帧表不驻此处——它们随
/// [`super::CallFrame`] 生命周期（池化 reset / 协程整体进出 record），
/// Frame 句柄的读/写经帧自身的 [`VarTable`]（此处的 Frame 臂保持
/// None 占位）；`Array` 臂步骤 3 接线。
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

    /// 按下标取命名空间表（步骤 3：搜索/戳条目的宿主名反查——目前
    /// 只有 0 号根表，每命名空间一张的拆分随后续步骤接入）。
    #[inline]
    pub(crate) fn table(&self, i: u32) -> Option<&VarTable> {
        self.tables.get(i as usize)
    }

    #[inline]
    pub(crate) fn root_mut(&mut self) -> &mut VarTable {
        &mut self.tables[0]
    }

    /// 句柄读（代数守卫在表内）。Frame 臂：帧表驻帧（见结构注），
    /// 经帧侧 [`VarTable::get_ref`] 访问——此处恒 None。Array 臂（步
    /// 骤 3）：宿主为全局（命名空间）表条目时经宿主 Var 的元素子表
    /// 访问（子表的归属/代数守卫照常）；帧宿主不可达（帧表驻帧——
    /// 元素句柄经帧侧访问），恒 None。
    #[allow(dead_code)] // 句柄 API——步骤 3 接线
    pub(crate) fn get(&self, r: &VarRef) -> Option<&Var> {
        match &r.table {
            TableId::Global(i) => self.tables.get(*i as usize).and_then(|t| t.get_ref(r)),
            TableId::Array(host) => match &host.table {
                TableId::Global(i) => self
                    .tables
                    .get(*i as usize)
                    .and_then(|t| t.get_ref(host))
                    .and_then(|v| v.elements())
                    .and_then(|t| t.get_ref(r)),
                _ => None,
            },
            _ => None,
        }
    }

    /// 句柄写（代数守卫同上；Array 臂同 [`Self::get`] 的寻址形）。
    #[allow(dead_code)] // 句柄 API——步骤 3 接线
    pub(crate) fn get_mut(&mut self, r: &VarRef) -> Option<&mut Var> {
        match &r.table {
            TableId::Global(i) => {
                self.tables.get_mut(*i as usize).and_then(|t| t.get_ref_mut(r))
            }
            TableId::Array(host) => {
                let host = host.clone();
                match host.table {
                    TableId::Global(i) => self
                        .tables
                        .get_mut(i as usize)
                        .and_then(|t| t.get_ref_mut(&host))
                        .and_then(|v| v.elements.as_deref_mut())
                        .and_then(|t| t.get_ref_mut(r)),
                    _ => None,
                }
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
        // Frame 臂：帧表驻帧（步骤 2），经帧侧 VarTable 访问——arena 恒 None。
        let fr = VarRef { table: TableId::Frame(0), slot: 0, gen: 0 };
        assert!(a.get(&fr).is_none());
        assert!(a.get_mut(&fr).is_none());
    }

    #[test]
    fn frame_table_identity_and_handle() {
        // 帧表身份：句柄携带 Frame(idx)；归属校验把别的表的槽号挡在门外。
        let mut t = VarTable::new_frame(2);
        let r = t.insert(Rc::from("x"), v("1"));
        assert!(matches!(r.table, TableId::Frame(2)));
        assert_eq!(t.get_ref(&r).map(|v| v.value.as_str()), Some("1"));
        assert_eq!(t.key_of(&r), Some("x"));
        // 换了身份的表（池化帧换深度复用）不认旧句柄。
        t.set_owner(TableId::Frame(5));
        assert!(t.get_ref(&r).is_none());
        // Global 句柄对帧表同样 miss。
        let g = VarRef { table: TableId::Global(0), slot: r.slot, gen: r.gen };
        assert!(t.get_ref(&g).is_none());
    }

    #[test]
    fn reset_clears_and_kills_stale_handles() {
        let mut t = VarTable::new_frame(0);
        let ra = t.insert(Rc::from("a"), v("1"));
        let rb = t.insert(Rc::from("b"), v("2"));
        let mut av = v("arr");
        av.flags.insert(VarFlags::ARRAY);
        av.elem_count = 3;
        t.insert(Rc::from("arr"), av);
        assert_eq!(t.len(), 3);
        assert_eq!(t.array_count(), 1);
        t.reset();
        // 全清：无活条目、无数组、顺序为空；分配容量保留（槽只增不减）。
        assert_eq!(t.len(), 0);
        assert_eq!(t.array_count(), 0);
        assert_eq!(t.order_entries().count(), 0);
        assert_eq!(t.slots.len(), 3);
        // 代数不冲突：旧句柄全部 miss（含从未删除过的条目——reset 逐槽
        // bump 代数，槽号复用也不会让旧柄复活）。
        assert!(t.get_ref(&ra).is_none());
        assert!(t.get_ref(&rb).is_none());
        assert!(t.key_of(&ra).is_none());
        // 重置后重建：新条目落复用槽、代数已递增，序语义从零开始。
        let ra2 = t.insert(Rc::from("a"), v("A"));
        assert_eq!(ra2.slot, ra.slot);
        assert_ne!(ra2.gen, ra.gen);
        assert_eq!(t.get_ref(&ra2).map(|v| v.value.as_str()), Some("A"));
        let seq: Vec<&str> = t.order_entries().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(seq, vec!["a"]);
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
        let want: Vec<String> = (0..n).map(|i| format!("k{i}")).collect::<Vec<_>>();
        assert_eq!(keys, want.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    }

    // -- D1 步骤 3：数组元素子表 ------------------------------------------

    /// mark_array 建子表（Array(宿主句柄) 身份）；元素读写经子表；宿
    /// 主摘除 → 子表随之回收（句柄全灭）；再建 → 新宿主句柄、新子表。
    #[test]
    fn array_subtable_lifecycle() {
        let mut t = VarTable::new();
        t.mark_array("a", Some(0));
        let host = t.resolve("a").expect("marked");
        assert!(matches!(host.table, TableId::Global(0)));
        let sub = t.get("a").unwrap().elements().expect("subtable");
        // 子表身份 = Array(宿主句柄)（同形同值——Eq 含 slot/gen）。
        assert_eq!(sub.owner, TableId::Array(Box::new(host.clone())));
        // 元素插入走子表自身形状（键 = 下标、插入序向量）。
        {
            let sub = t.get_mut("a").unwrap().elements_mut().unwrap();
            let e1 = sub.insert(Rc::from("k1"), v("1"));
            let e2 = sub.insert(Rc::from("k2"), v("2"));
            assert_eq!(
                sub.order_entries().map(|(k, _)| k.as_ref()).collect::<Vec<_>>(),
                vec!["k1", "k2"]
            );
            // 删除重建取新位（tclsh 表序语义，与宿主表同机制）。
            assert!(sub.remove("k1").is_some());
            sub.insert(Rc::from("k1"), v("1b"));
            assert_eq!(
                sub.order_entries().map(|(k, _)| k.as_ref()).collect::<Vec<_>>(),
                vec!["k2", "k1"]
            );
            assert!(t.get_ref(&e1).is_none());
            assert_eq!(t.get_ref(&e2).map(|x| x.value.as_str()), Some("2"));
        }
        // 宿主摘除：子表回收，元素句柄全灭。
        assert!(t.remove("a").is_some());
        // 再建：新代数句柄、空子表。
        t.mark_array("a", Some(0));
        let host2 = t.resolve("a").unwrap();
        assert_ne!(host.gen, host2.gen);
        assert_eq!(t.get("a").unwrap().elements().unwrap().len(), 0);
    }

    /// land_value（值落地）不得清掉子表与元素计数；unmark 撤位回收子
    /// 表。arena Array 臂的句柄读（宿主全局表 → 子表 → 元素）。
    #[test]
    fn land_value_preserves_subtable_and_arena_array_arm() {
        let mut a = VarArena::new();
        a.root_mut().mark_array("a", Some(0));
        let host = a.root().resolve("a").unwrap();
        {
            let sub = a.root_mut().get_mut("a").unwrap().elements_mut().unwrap();
            let er = sub.insert(Rc::from("k"), v("9"));
            // arena Array 臂：元素句柄读经宿主。
            assert_eq!(a.get(&er).map(|x| x.value.as_str()), Some("9"));
            let mut er2 = er.clone();
            er2.gen += 1;
            assert!(a.get(&er2).is_none()); // 代数守卫
            let mut fr = er;
            fr.table = TableId::Frame(0);
            assert!(a.get(&fr).is_none()); // 身份守卫
        }
        // 值落地（基键空值 → 有值）：子表 + 计数保持。
        let (r, is_new) = a.root_mut().land_value("a".to_string(), Value::empty());
        assert_eq!(r.slot, host.slot);
        assert!(is_new); // 无值占位上值 = 新建序位
        let var = a.root().get("a").unwrap();
        assert_eq!(var.elem_count, 0);
        assert_eq!(var.elements().unwrap().get("k").unwrap().value.as_str(), "9");
        // 撤位：子表回收、计数清零。
        a.root_mut().unmark_array("a");
        let var = a.root().get("a").unwrap();
        assert!(!var.is_array());
        assert!(var.elements.is_none());
        assert_eq!(var.elem_count, 0);
    }

    /// 历史入档/压缩：曾插入键各一次（末次出现位）；删除键落档后桶
    /// 轨迹回落（与共享日志压缩同形）；reset 全清。
    #[test]
    fn subtable_history_and_reset() {
        let mut t = VarTable::new();
        t.mark_array("a", Some(0));
        for i in 0..100u32 {
            let k = format!("k{i}");
            let sub = t.get_mut("a").unwrap().elements_mut().unwrap();
            sub.insert(Rc::from(k.as_str()), v("x"));
            sub.history_push(&k);
        }
        for i in 0..100u32 {
            let k = format!("k{i}");
            let sub = t.get_mut("a").unwrap().elements_mut().unwrap();
            sub.remove(&k);
        }
        {
            let sub = t.get_mut("a").unwrap().elements_mut().unwrap();
            sub.insert(Rc::from("live"), v("L"));
            sub.history_push("live");
            // 压缩触发后：删除键落档，只余活键。
            assert_eq!(sub.history_keys().collect::<Vec<_>>(), vec!["live"]);
            let seq: Vec<&str> = sub.order_entries().map(|(k, _)| k.as_ref()).collect();
            assert_eq!(seq, vec!["live"]);
        }
        // reset：子表清空（帧池重置语义——元素句柄全灭）。
        let mut f = VarTable::new_frame(0);
        f.mark_array("a", Some(0));
        let h = f.resolve("a").unwrap();
        let er = {
            let sub = f.get_mut("a").unwrap().elements_mut().unwrap();
            sub.insert(Rc::from("k"), v("1"))
        };
        assert!(f.get_ref(&er).is_some());
        f.reset();
        assert!(f.get_ref(&er).is_none());
        assert!(f.get_ref(&h).is_none());
        assert!(f.get("a").is_none());
    }
}
