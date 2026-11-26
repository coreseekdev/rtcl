//! Value type for rtcl - implements "everything is a string" philosophy
//!
//! Tcl values can have an internal representation for efficiency
//! while still being representable as strings.
//!
//! # Memory model
//!
//! Values are reference-counted (`VPtr`, a hand-rolled `Rc<ValueInner>`)
//! with copy-on-write semantics, mirroring jimtcl's `refCount` /
//! `Jim_DuplicateObj()` model.
//!
//! - `Value::clone()` is O(1) — simply increments the reference count.
//! - Mutation (e.g. list append) goes through copy-on-write
//!   (`Value::heap_make_mut`), which deep-copies only when the value is
//!   shared (reference count > 1).
//! - The boxes come from a thread-local free list (jimtcl's free-list
//!   pointers, which the original Rust port dropped in favour of the
//!   allocator): a value whose last reference dies pushes its box back,
//!   and the next value to be born reuses it.  After the interpreter's
//!   working set is warm, per-value malloc/free traffic — the allocator
//!   share the fib/fe_bench profiles kept showing — goes away; the
//!   PAYLOADS (string buffers, list/dict vecs) are still dropped and
//!   re-allocated normally.
//! - Structured data (lists, dicts) are cached inside `InternalRep` to
//!   avoid repeated string parse / serialize round-trips.
//! - The string representation is lazily generated: after mutating the
//!   internal representation, the string is invalidated and only
//!   regenerated when `as_str()` is called.

use core::fmt;
use core::str::FromStr;
use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;
use std::rc::Rc;

use indexmap::IndexMap;
use num_bigint::BigInt;
use num_traits::ToPrimitive as _;
use smallvec::SmallVec;

// ── dict-key hashing ───────────────────────────────────────────
// Unkeyed multiply-mix hasher for dict keys — the same shape and
// rationale as the interpreter's variable tables (rtcl-core's
// interp/varmap.rs: ~2ns vs ~20ns per short-key probe; tclsh hashes
// script-provided dict keys unkeyed too).  Neither dict variant's
// ITERATION order depends on it: `Ordered` keeps insertion order,
// `Unordered`'s order was never specified.  Duplicated here because
// the dependency edge runs rtcl-core -> rtcl-vm.
const FX_SEED: u64 = 0x517c_c1b7_2722_0a95;

/// Implementation detail of [`DictMap`]: unkeyed multiply-mix hasher.
/// Exposed only because it parameterizes `DictMap`'s map types; do not
/// rely on its exact hash values.
#[derive(Default, Clone)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

// Only `write` + `write_u8` are exercised (`Hash for str` decomposes to
// those) — same note as varmap.rs.
//
// Unlike the variable tables, dicts grow to 100k+ script-chosen keys, and
// hashbrown picks the bucket from the LOW bits of the finished hash.  A
// raw multiply-mix leaves those low bits dependent only on the first ~2
// bytes (a shared prefix like "key<i>" makes every key congruent mod the
// table mask — 300k inserts into one probe chain, quadratic).  `finish`
// therefore runs murmur3's fmix64 avalanche over the accumulated word:
// a bijection (distinct keys stay distinct) whose low bits depend on ALL
// input bytes.  ~2 multiplies — still far below SipHash.
impl core::hash::Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        let mut x = self.hash;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        x ^= x >> 33;
        x
    }

    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rem) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
}

type FastBuild = std::hash::BuildHasherDefault<FxHasher>;
type FastIndexMap = IndexMap<String, Value, FastBuild>;
type FastHashMap = HashMap<String, Value, FastBuild>;

/// Maximum inline string length before heap allocation
const INLINE_SIZE: usize = 23;

/// Internal representation of a value.
///
/// Mirrors jimtcl's `internalRep` union — a cached typed view of the
/// underlying string value.
#[derive(Debug)]
pub enum InternalRep {
    /// Integer representation
    Int(i64),
    /// 任意精度整数（D5）：i63 立即数与 i64 堆 Int 装不下的精确值。
    /// `Rc` 让 Clone 摊薄为引用计数加一——bignum 值在列表/字典克隆与
    /// 栈搬运间转手是常态，深拷 BigInt 的位数开销只配留给真正的运算。
    /// 字符串形态（canonical 十进制）仍惰性按需生成。
    Big(Rc<BigInt>),
    /// Floating point representation
    Float(f64),
    /// Boolean representation
    Bool(bool),
    /// Cached list of values (avoids re-parsing the string)
    List(Vec<Value>),
    /// Cached dict — supports both ordered (insertion-order) and unordered modes.
    Dict(DictMap),
    /// 宿主字符串池索引（lazy-string §6）：JS 起源的串保持宿主原生形态
    /// （wasm32 = externref JsString，池在 rtcl-wasm 的 `JsStringBacking`），
    /// 首次字节级语义经 `as_str` choke point 物化；Tcl→宿主反向穿越缓存
    /// 池索引（Dual 态）。无人注册背板时此 variant 无人构造（运行期不出
    /// 现，零成本——注册表即 `lazy-string` 开关，见 `host_string.rs`）。
    /// `Clone`/`Drop` 是手工实现：池槽位引用计数跟随 ValueInner 的 COW 深
    /// 拷贝与回收。
    HostStr(u32),
    /// No internal representation yet
    None,
}

impl Clone for InternalRep {
    fn clone(&self) -> Self {
        match self {
            InternalRep::Int(n) => InternalRep::Int(*n),
            // Rc 份额复制：引用计数加一，位数不拷。
            InternalRep::Big(b) => InternalRep::Big(Rc::clone(b)),
            InternalRep::Float(n) => InternalRep::Float(*n),
            InternalRep::Bool(b) => InternalRep::Bool(*b),
            InternalRep::List(v) => InternalRep::List(v.clone()),
            InternalRep::Dict(m) => InternalRep::Dict(m.clone()),
            InternalRep::None => InternalRep::None,
            InternalRep::HostStr(idx) => {
                // COW 深拷贝：两个 ValueInner 即将引用同一池槽位——加池引
                // 用，配平各自的 release。背板缺位时（HostStr 只在有背板
                // 时构造，此臂不可达）跳过；槽位泄漏给显式 acquire 不归还
                // 的错误实现会在池侧暴露。
                crate::host_string::with_backing(|b| b.acquire(*idx));
                InternalRep::HostStr(*idx)
            }
        }
    }
}

impl Drop for InternalRep {
    fn drop(&mut self) {
        if let InternalRep::HostStr(idx) = self {
            // 值回收（VPtr 末主 drop_in_place）或 rep 覆写（set_int_rep
            // 等赋值丢弃旧 rep）都汇到这里——即状态机的 ──mutation──→
            // NativeOnly 转移，池引用归还。
            crate::host_string::with_backing(|b| b.release(*idx));
        }
    }
}

// ── DictMap ────────────────────────────────────────────────────

/// Dictionary map supporting both ordered (insertion-order) and unordered modes.
///
/// - `Ordered`: backed by `IndexMap`, preserves insertion order (standard Tcl dict semantics)
/// - `Unordered`: backed by `HashMap`, no order guarantee (faster for large dicts)
#[derive(Debug, Clone)]
pub enum DictMap {
    Ordered(FastIndexMap),
    Unordered(FastHashMap),
}

impl Default for DictMap {
    fn default() -> Self { DictMap::Ordered(FastIndexMap::default()) }
}

impl DictMap {
    pub fn ordered() -> Self { DictMap::Ordered(FastIndexMap::default()) }
    pub fn unordered() -> Self { DictMap::Unordered(FastHashMap::default()) }

    pub fn ordered_with_capacity(cap: usize) -> Self {
        DictMap::Ordered(FastIndexMap::with_capacity_and_hasher(cap, FastBuild::default()))
    }
    pub fn unordered_with_capacity(cap: usize) -> Self {
        DictMap::Unordered(FastHashMap::with_capacity_and_hasher(cap, FastBuild::default()))
    }

    pub fn is_ordered(&self) -> bool { matches!(self, DictMap::Ordered(_)) }

    /// Build a new DictMap with the given ordering mode from an iterator.
    pub fn from_iter_with_order<I>(ordered: bool, iter: I) -> Self
    where
        I: IntoIterator<Item = (String, Value)>,
    {
        if ordered {
            DictMap::Ordered(iter.into_iter().collect())
        } else {
            DictMap::Unordered(iter.into_iter().collect())
        }
    }

    /// Create an empty DictMap matching the ordering mode of `self`.
    pub fn empty_like(&self, capacity: usize) -> Self {
        if self.is_ordered() {
            DictMap::ordered_with_capacity(capacity)
        } else {
            DictMap::unordered_with_capacity(capacity)
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self { DictMap::Ordered(m) => m.get(key), DictMap::Unordered(m) => m.get(key) }
    }

    /// Mutable access to one entry — the nested `dict set` fast path's
    /// in-place walk needs `&mut Value` handles held by the parent map
    /// (child COW through `as_dict_mut` then lands in the same entry).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self { DictMap::Ordered(m) => m.get_mut(key), DictMap::Unordered(m) => m.get_mut(key) }
    }

    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        match self { DictMap::Ordered(m) => m.insert(key, value), DictMap::Unordered(m) => m.insert(key, value) }
    }

    /// Remove a key (preserves order for ordered maps).
    pub fn shift_remove(&mut self, key: &str) -> Option<Value> {
        match self { DictMap::Ordered(m) => m.shift_remove(key), DictMap::Unordered(m) => m.remove(key) }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        match self { DictMap::Ordered(m) => m.contains_key(key), DictMap::Unordered(m) => m.contains_key(key) }
    }

    pub fn len(&self) -> usize {
        match self { DictMap::Ordered(m) => m.len(), DictMap::Unordered(m) => m.len() }
    }

    pub fn is_empty(&self) -> bool {
        match self { DictMap::Ordered(m) => m.is_empty(), DictMap::Unordered(m) => m.is_empty() }
    }

    pub fn keys(&self) -> DictKeys<'_> {
        match self {
            DictMap::Ordered(m) => DictKeys::Ordered(m.keys()),
            DictMap::Unordered(m) => DictKeys::Unordered(m.keys()),
        }
    }

    pub fn values(&self) -> DictValues<'_> {
        match self {
            DictMap::Ordered(m) => DictValues::Ordered(m.values()),
            DictMap::Unordered(m) => DictValues::Unordered(m.values()),
        }
    }

    pub fn iter(&self) -> DictIter<'_> {
        match self {
            DictMap::Ordered(m) => DictIter::Ordered(m.iter()),
            DictMap::Unordered(m) => DictIter::Unordered(m.iter()),
        }
    }
}

impl Extend<(String, Value)> for DictMap {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        match self { DictMap::Ordered(m) => m.extend(iter), DictMap::Unordered(m) => m.extend(iter) }
    }
}

impl IntoIterator for DictMap {
    type Item = (String, Value);
    type IntoIter = DictIntoIter;
    fn into_iter(self) -> Self::IntoIter {
        match self {
            DictMap::Ordered(m) => DictIntoIter::Ordered(m.into_iter()),
            DictMap::Unordered(m) => DictIntoIter::Unordered(m.into_iter()),
        }
    }
}

impl<'a> IntoIterator for &'a DictMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = DictIter<'a>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

// ── Iterator types ─────────────────────────────────────────────

pub enum DictIter<'a> {
    Ordered(indexmap::map::Iter<'a, String, Value>),
    Unordered(std::collections::hash_map::Iter<'a, String, Value>),
}
impl<'a> Iterator for DictIter<'a> {
    type Item = (&'a String, &'a Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictIter::Ordered(i) => i.next(), DictIter::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictIter::Ordered(i) => i.size_hint(), DictIter::Unordered(i) => i.size_hint() }
    }
}

pub enum DictIntoIter {
    Ordered(indexmap::map::IntoIter<String, Value>),
    Unordered(std::collections::hash_map::IntoIter<String, Value>),
}
impl Iterator for DictIntoIter {
    type Item = (String, Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictIntoIter::Ordered(i) => i.next(), DictIntoIter::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictIntoIter::Ordered(i) => i.size_hint(), DictIntoIter::Unordered(i) => i.size_hint() }
    }
}

pub enum DictKeys<'a> {
    Ordered(indexmap::map::Keys<'a, String, Value>),
    Unordered(std::collections::hash_map::Keys<'a, String, Value>),
}
impl<'a> Iterator for DictKeys<'a> {
    type Item = &'a String;
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictKeys::Ordered(i) => i.next(), DictKeys::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictKeys::Ordered(i) => i.size_hint(), DictKeys::Unordered(i) => i.size_hint() }
    }
}

pub enum DictValues<'a> {
    Ordered(indexmap::map::Values<'a, String, Value>),
    Unordered(std::collections::hash_map::Values<'a, String, Value>),
}
impl<'a> Iterator for DictValues<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictValues::Ordered(i) => i.next(), DictValues::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictValues::Ordered(i) => i.size_hint(), DictValues::Unordered(i) => i.size_hint() }
    }
}

// ── D4：字符串 rep = 字节 + 惰性 char_len ─────────────────────

/// Sentinel of [`StringRep::char_len`]: "not computed yet"（字符数达到
/// usize::MAX 需要 2^63 字节的串，物理不可达）。
const CHAR_LEN_UNKNOWN: usize = usize::MAX;

/// The string representation of a heap box: the lazily materialized bytes
/// and the lazily computed UTF-8 char length, bound into ONE unit (D4).
///
/// 绑定的理由（审计 §3.4 / HANDOFF §3.4「每个改写入口都要把计数作废」
/// 的结构性解法）：改写入口清点的结论是——字节的**原地**写全仓只有
/// `append_str_in_place` 一处（增量维护），其余三处让字节整体失效的
/// 站点（`as_list_mut`/`as_dict_mut`/`set_int_rep`）写的都是
/// `string = StringRep::new()` 整体换新。字节与计数捆绑后，「换了字节
/// 忘了作废计数」在结构上不可能发生：计数要么随新字节归 UNKNOWN，要么
/// 在 append 里跟字节一起长。此外全部其余改写入口（str_replace/map/
/// trim 族/case 族/reverse/range 截断/format/binary/concat/…）产出的
/// 都是新箱（`from_str`/`from_string`），缓存天然从 UNKNOWN 起步。
///
/// 共享箱（rc > 1）写 `char_len`（`Cell`，&self 可写）是安全的：字节
/// 受 COW 保护不变（`heap_make_mut` 先深拷），缓存是字节的纯函数，
/// 各共享者读到的字节相同，计数亦同。
///
/// 代价：`ValueInner` 每箱 +8 字节（box 池上限 8192 箱 → 最坏 +64KB，
/// 热集后实际驻留远小于此）。`Value` 本体的 8 字节标记字不动（红线：
/// 审计 §3.4 与 Agent.md——标记字余量不进位，指针掩码会污染全部热径）。
#[derive(Debug, Clone)]
struct StringRep {
    /// Bytes — lazily materialized via `OnceCell`.  Empty cell means it
    /// will be generated on first `as_str()` access from `rep` (like
    /// jimtcl's `bytes == NULL`).
    bytes: OnceCell<SmallVec<[u8; INLINE_SIZE]>>,
    /// UTF-8 码点数（D4）：CHAR_LEN_UNKNOWN = 未算。语义按 rtcl 既有
    /// 口径 = `chars().count()`（T7 的 SWAR 连续字节计数：合法 UTF-8
    /// 中字符数 ≡ 非连续字节数，两者逐位同值）。
    char_len: Cell<usize>,
}

impl StringRep {
    /// Fresh cell with no bytes and no count（改写失效与惰性构造共用）。
    #[inline]
    fn new() -> Self {
        StringRep { bytes: OnceCell::new(), char_len: Cell::new(CHAR_LEN_UNKNOWN) }
    }

    /// Fresh cell with known bytes; the count stays lazy (a big literal
    /// nobody length-queries pays no eager scan).
    #[inline]
    fn with_bytes(b: SmallVec<[u8; INLINE_SIZE]>) -> Self {
        StringRep { bytes: OnceCell::from(b), char_len: Cell::new(CHAR_LEN_UNKNOWN) }
    }
}

/// The number of UTF-8 characters in `s` — its byte length where all its
/// bytes are characters（ASCII 一步到位），otherwise T7's SWAR
/// continuation count subtracted from the byte length（24KB 两字节文本
/// 3.1µs 一档，重算地板）。
#[inline]
pub(crate) fn char_len_of(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.len() - count_continuations(s.as_bytes())
    }
}

/// UTF-8 continuation bytes (0x80..0xBF) in `bytes`, eight at a time:
/// per u64 word, `(b ^ 0x80) & 0xC0 == 0` marks a continuation byte,
/// and the strlen-style zero-byte mask `(w - 0x0101..) & !w & 0x8080..`
/// marks exactly those (a borrow entering a non-zero byte keeps the
/// marker clear: `y - 1 ≥ 0x80 ∧ y ≤ 0x7F` is impossible).  T7 实证
/// 形（bench/BASELINE「P15 string last 三处」）。
pub(crate) fn count_continuations(bytes: &[u8]) -> usize {
    const HI: u64 = 0x8080_8080_8080_8080;
    const LO: u64 = 0x0101_0101_0101_0101;
    const TOP2: u64 = 0xC0C0_C0C0_C0C0_C0C0;
    let (chunks, rem) = bytes.as_chunks::<8>();
    let mut cont = 0usize;
    for &c in chunks {
        let word = u64::from_le_bytes(c);
        let w = (word ^ HI) & TOP2;
        cont += ((w.wrapping_sub(LO)) & !w & HI).count_ones() as usize;
    }
    for &b in rem {
        cont += ((b & 0xC0) == 0x80) as usize;
    }
    cont
}

/// D4 等价红线自检（release 构建零成本——`cfg!` 常量假，整体消去）：
/// debug 构建里每次 `char_len` 读取都与当场重算对拍，缓存失效漏网
/// （改写入口置无效不全）当场断言，而不是等输出漂移。debug 测试套件
/// （workspace 1481 项）因此全程带验。
#[inline]
fn d4_selfcheck(v: &Value, cached: usize) {
    if cfg!(debug_assertions) {
        let fresh = char_len_of(v.as_str());
        assert_eq!(fresh, cached, "D4 char_len 缓存与重算不一致（缓存失效漏网）");
    }
}

/// Inner data shared via `Rc`.
///
/// Corresponds to jimtcl's `Jim_Obj` minus the free-list pointers —
/// Rust's allocator handles recycling.
#[derive(Debug)]
struct ValueInner {
    /// String representation (bytes + lazy char length, D4) — bytes are
    /// lazily materialized via `OnceCell`.
    string: StringRep,
    /// Cached typed representation.
    rep: InternalRep,
    /// Reference count (jimtcl's `refCount`).  Owned by [`VPtr`]; a
    /// cloned payload always starts at 1 (the clone owns its box).
    rc: Cell<usize>,
}

impl Clone for ValueInner {
    fn clone(&self) -> Self {
        ValueInner {
            string: self.string.clone(),
            rep: self.rep.clone(),
            rc: Cell::new(1),
        }
    }
}

// ── the recycled reference-counted box ────────────────────────

/// Recycled-box reference-counted pointer to a [`ValueInner`] — a
/// hand-rolled `Rc` whose dead boxes go to a thread-local free list
/// instead of the allocator (jimtcl's free-list pointers).  The pointer
/// is the SAME 8 bytes an `Rc` was, so [`Value`]'s layout is unchanged;
/// `clone`/`drop` reproduce the `Rc` semantics exactly, and `Deref`
/// keeps every accessor working
/// unchanged.  Deliberately `!Send`/`!Sync` (as `Rc` was): a value never
/// crosses threads.
struct VPtr(NonNull<ValueInner>);

/// Recycled-box cap.  Beyond this the box is genuinely freed; 8192
/// boxes ≈ 0.6MB of hot ValueInner memory.
const FREELIST_CAP: usize = 8192;

struct FreeList(std::cell::RefCell<Vec<NonNull<ValueInner>>>);

impl Drop for FreeList {
    /// Thread exit: the recycled boxes are bare allocations (their payloads
    /// were already dropped on recycle).  Returning them to the allocator
    /// avoids leaking up to FREELIST_CAP boxes per thread.
    fn drop(&mut self) {
        for p in self.0.borrow_mut().drain(..) {
            unsafe {
                std::alloc::dealloc(
                    p.as_ptr().cast::<u8>(),
                    std::alloc::Layout::new::<ValueInner>(),
                );
            }
        }
    }
}

thread_local! {
    static FREELIST: FreeList = const { FreeList(std::cell::RefCell::new(Vec::new())) };
}

impl VPtr {
    fn new(inner: ValueInner) -> VPtr {
        let recycled = FREELIST.try_with(|fl| fl.0.borrow_mut().pop());
        match recycled {
            Ok(Some(ptr)) => {
                // The box's payload was dropped when it was recycled; the
                // memory itself is still a live allocation.
                unsafe { std::ptr::write(ptr.as_ptr(), inner) };
                VPtr(ptr)
            }
            _ => VPtr(NonNull::from(Box::leak(Box::new(inner)))),
        }
    }
}

impl Clone for VPtr {
    fn clone(&self) -> Self {
        unsafe {
            let p = self.0.as_ptr();
            (*p).rc.set((*p).rc.get() + 1);
        }
        VPtr(self.0)
    }
}

impl Drop for VPtr {
    fn drop(&mut self) {
        unsafe {
            let p = self.0.as_ptr();
            let n = (*p).rc.get();
            if n > 1 {
                (*p).rc.set(n - 1);
                return;
            }
            // Last owner: free the payload (string buffer, list/dict
            // vecs), then recycle the box itself.
            std::ptr::drop_in_place(p);
            let pushed = FREELIST.try_with(|fl| {
                let mut fl = fl.0.borrow_mut();
                if fl.len() < FREELIST_CAP {
                    fl.push(self.0);
                    true
                } else {
                    false
                }
            });
            if pushed != Ok(true) {
                // Freelist full — or already torn down at thread exit
                // (try_with fails there): free the BOX for real.  The
                // payload was already dropped above, so this must be a
                // bare dealloc — `drop(Box::from_raw(p))` would run
                // drop_in_place a SECOND time (the double-free the
                // gen_dict corpus caught at the freelist-cap threshold).
                std::alloc::dealloc(
                    p.cast::<u8>(),
                    std::alloc::Layout::new::<ValueInner>(),
                );
            }
        }
    }
}

impl Deref for VPtr {
    type Target = ValueInner;
    fn deref(&self) -> &ValueInner {
        unsafe { self.0.as_ref() }
    }
}

impl fmt::Debug for VPtr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A Tcl value - "everything is a string"
///
/// Cloning a `Value` is **O(1)** — for the common case it is not even
/// a refcount: the word is a tagged repr, low bit set = an **immediate
/// integer** (`i63`, `[-2^62, 2^62)` — clone is a copy, drop is
/// nothing, as_int is an untag, from_int never allocates; the
/// full-i64 extremes fall back to a heap box with the Int rep).  Low
/// bit clear = a heap box (recycled `VPtr`), refcounted with COW.
///
/// The one interior transition is *materialization*: an immediate
/// whose string is first demanded upgrades itself to a heap box
/// through the `Cell` and returns the borrow into that box — one-way
/// and idempotent, so a `&str` from `as_str` stays valid for `&self`'s
/// lifetime (clones bump the box's count; every other `&self` method
/// is read-only).  `PhantomData<*mut _>` keeps `Value` !Send/!Sync,
/// the same auto-trait surface `Rc` had.
pub struct Value {
    bits: Cell<u64>,
    _not_send: PhantomData<*mut ValueInner>,
}

impl Value {
    /// The unset-slot sentinel: bits == 0 is unreachable for valid values
    /// (immediates have bit0 set; heap pointers are non-null), giving
    /// `slots: Vec<Value>` an 8-byte element with an in-band "unset"
    /// state — half the memory traffic of `Vec<Option<Value>>`.
    ///
    /// A constructor fn rather than a `const` on purpose: `Value` wraps a
    /// `Cell`, so an interior-mutable `const UNSET` would read as shared
    /// mutable state.  It never is — a const inlines a fresh copy per use
    /// and `Value` is !Send/!Sync — and the fn makes that fresh-copy-per-use
    /// semantics explicit (there is no canonical UNSET instance to mutate).
    #[inline]
    #[must_use]
    pub const fn unset() -> Value {
        Value { bits: Cell::new(0), _not_send: PhantomData }
    }

    #[inline]
    pub fn is_unset(&self) -> bool {
        self.bits.get() == 0
    }
    #[inline]
    pub fn is_imm(&self) -> bool {
        self.bits.get() & 1 == 1
    }
    #[inline]
    pub fn imm(&self) -> i64 {
        (self.bits.get() as i64) >> 1
    }
    #[inline]
    pub fn from_imm(n: i64) -> Value {
        Value::with_bits(((n as u64) << 1) | 1)
    }
    #[inline]
    pub(crate) fn set_imm(&self, n: i64) {
        self.bits.set(((n as u64) << 1) | 1);
    }
    /// # Safety
    /// `bits` must be a heap repr (low bit clear) from a live box.
    #[inline]
    unsafe fn heap_inner(&self) -> &ValueInner {
        &*(self.bits.get() as *const ValueInner)
    }
    #[inline]
    fn heap_ptr(&self) -> *mut ValueInner {
        self.bits.get() as *mut ValueInner
    }
    #[inline]
    fn with_bits(bits: u64) -> Value {
        Value { bits: Cell::new(bits), _not_send: PhantomData }
    }
    /// The heap COW path shared by the mutators: deep-copy when shared.
    #[inline]
    fn heap_make_mut(&mut self) -> &mut ValueInner {
        let p = self.heap_ptr();
        unsafe {
            if (*p).rc.get() > 1 {
                let cloned = (*p).clone(); // payload deep-copy, rc starts at 1
                let fresh = VPtr::new(cloned);
                let new_bits = fresh.0.as_ptr();
                std::mem::forget(fresh); // ownership transfers into `bits`
                (*p).rc.set((*p).rc.get() - 1);
                self.bits.set(new_bits as u64);
                &mut *new_bits
            } else {
                &mut *p
            }
        }
    }
    /// Take ownership of a freshly built heap box as a heap `Value`.
    #[inline]
    fn from_vptr(v: VPtr) -> Value {
        let bits = v.0.as_ptr() as u64;
        std::mem::forget(v);
        Value::with_bits(bits)
    }
    /// A heap `Value` sharing `other`'s box (refcount bump).
    #[inline]
    fn from_heap_clone(other: &VPtr) -> Value {
        unsafe {
            (*other.0.as_ptr()).rc.set((*other.0.as_ptr()).rc.get() + 1);
        }
        Value::with_bits(other.0.as_ptr() as u64)
    }
}

impl Clone for Value {
    #[inline]
    fn clone(&self) -> Self {
        let bits = self.bits.get();
        // Immediates copy; the UNSET sentinel (bits 0) copies; heap words
        // bump the refcount.
        if bits & 1 == 1 || bits == 0 {
            return Value::with_bits(bits);
        }
        unsafe {
            let p = bits as *mut ValueInner;
            (*p).rc.set((*p).rc.get() + 1);
        }
        Value::with_bits(bits)
    }
}

impl Drop for Value {
    /// Two inlined tiers before any call: the tag check (immediates and
    /// the UNSET sentinel own no heap box) and a plain refcount decrement
    /// for a shared box.  Every discarded immediate — each loop's popped
    /// `incr`/`set`/`lappend` result — used to pay a `drop_glue` call;
    /// only the last-owner free is outlined.
    #[inline]
    fn drop(&mut self) {
        let bits = self.bits.get();
        // Immediates and the UNSET sentinel own no heap box.
        if bits & 1 == 1 || bits == 0 {
            return;
        }
        unsafe {
            let p = bits as *mut ValueInner;
            let n = (*p).rc.get();
            if n > 1 {
                (*p).rc.set(n - 1);
                return;
            }
        }
        self.drop_last_owner();
    }
}

impl Value {
    /// The last-owner arm of [`Drop`], outlined: free the payload (string
    /// buffer, list/dict vecs), then recycle the box itself.
    ///
    /// Teardown recurses natively while the nesting is shallow — that is
    /// the fast path for the usual wide lists and costs no extra memory —
    /// and switches to the iterative form ([`dismantle`]) past
    /// [`DROP_RECURSION_LIMIT`] levels: the recursive glue nests once per
    /// level, and 160,000 levels of `set l [list $l]` followed by
    /// `unset l` ran the native stack out (tclsh frees a million).
    #[inline(never)]
    fn drop_last_owner(&mut self) {
        let bits = self.bits.get();
        if bits & 1 == 1 || bits == 0 {
            // Defensive: only reached with a heap box (the inline tier
            // returns first); kept so the outlined arm is self-contained.
            return;
        }
        unsafe {
            let p = bits as *mut ValueInner;
            // `try_with`: at thread exit the counter is gone, and nothing
            // may recurse then either.
            let deep = DROP_DEPTH
                .try_with(|d| {
                    let depth = d.get();
                    d.set(depth + 1);
                    depth >= DROP_RECURSION_LIMIT
                })
                .unwrap_or(true);
            if !deep {
                std::ptr::drop_in_place(p);
                let _ = DROP_DEPTH.try_with(|d| d.set(d.get() - 1));
                recycle(p);
                return;
            }
            let mut work: Vec<Value> = Vec::new();
            dismantle(p, &mut work);
            while let Some(mut child) = work.pop() {
                // Neutralize the handle first: its box is about to be
                // recycled, and the handle's own Drop must not touch it.
                let cb = child.bits.replace(0);
                if cb & 1 == 1 || cb == 0 {
                    continue;
                }
                let q = cb as *mut ValueInner;
                let n = (*q).rc.get();
                if n > 1 {
                    (*q).rc.set(n - 1);
                    continue;
                }
                dismantle(q, &mut work);
            }
        }
    }
}

thread_local! {
    /// Native depth of the recursive teardown in progress on this thread.
    static DROP_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Native recursion allowed during teardown before the children start
/// going to the work vector (see [`Value::drop_last_owner`]).
const DROP_RECURSION_LIMIT: usize = 64;

/// Return a dead box whose payload is already dropped to the free list,
/// or to the allocator when the list is full (or already torn down at
/// thread exit).
unsafe fn recycle(p: *mut ValueInner) {
    let pushed = FREELIST.try_with(|fl| {
        let mut fl = fl.0.borrow_mut();
        if fl.len() < FREELIST_CAP {
            fl.push(NonNull::new_unchecked(p));
            true
        } else {
            false
        }
    });
    if pushed != Ok(true) {
        // Freelist full — or already torn down at thread exit
        // (try_with fails there): free the BOX for real. The payload was
        // already dropped above, so this must be a bare dealloc —
        // `drop(Box::from_raw(p))` would run drop_in_place a SECOND time
        // (the double-free the gen_dict corpus once caught at the
        // freelist-cap threshold).
        std::alloc::dealloc(
            p.cast::<u8>(),
            std::alloc::Layout::new::<ValueInner>(),
        );
    }
}

/// The last owner of the box at `p` lets go, without recursing: move the
/// box's list elements and dict values to `work` (the caller then gives
/// each of them the same treatment), drop what is left of the payload,
/// and recycle the box.
///
/// # Safety
/// `p` is a live box whose refcount is 1; nobody else holds it.
unsafe fn dismantle(p: *mut ValueInner, work: &mut Vec<Value>) {
    // Children out first, so `drop_in_place` below never enters a
    // `Vec<Value>`/dict drop that would recurse into them.
    match &mut (*p).rep {
        InternalRep::List(items) => work.append(&mut std::mem::take(items)),
        InternalRep::Dict(map) => match std::mem::take(map) {
            DictMap::Ordered(m) => work.extend(m.into_iter().map(|(_, v)| v)),
            DictMap::Unordered(m) => work.extend(m.into_iter().map(|(_, v)| v)),
        },
        _ => {}
    }
    std::ptr::drop_in_place(p);
    recycle(p);
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_imm() {
            return write!(f, "Value(imm {})", self.imm());
        }
        f.debug_struct("Value").finish_non_exhaustive()
    }
}

// ── Cached singletons ──────────────────────────────────────────
//
// The VM's hottest paths (`PushEmpty`, `PushBool`) create the same values
// millions of times.  By caching them as thread-local `Rc` singletons we
// eliminate per-use `Rc::new` + `OnceCell::from` overhead — `Value::clone()`
// becomes a simple reference-count increment.

thread_local! {
    static CACHED_EMPTY: VPtr = VPtr::new(ValueInner {
        string: StringRep::with_bytes(SmallVec::new()),
        rep: InternalRep::None,
        rc: Cell::new(1),
    });

    static CACHED_BOOL_TRUE: VPtr = VPtr::new(ValueInner {
        string: StringRep::with_bytes(SmallVec::from_slice(b"1")),
        rep: InternalRep::Bool(true),
        rc: Cell::new(1),
    });

    static CACHED_BOOL_FALSE: VPtr = VPtr::new(ValueInner {
        string: StringRep::with_bytes(SmallVec::from_slice(b"0")),
        rep: InternalRep::Bool(false),
        rc: Cell::new(1),
    });
}

impl Default for Value {
    fn default() -> Self {
        Self::empty()
    }
}

impl Value {
    /// Create an empty value
    pub fn empty() -> Self {
        CACHED_EMPTY.with(Value::from_heap_clone)
    }

    /// Create a value from a string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
                string: StringRep::with_bytes(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::None,
                rc: Cell::new(1),
            }))
    }

    /// Create a value from an owned string — the buffer moves into the
    /// value (`from_str` copies it).
    #[allow(clippy::should_implement_trait)]
    pub fn from_string(s: String) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
                string: StringRep::with_bytes(SmallVec::from_vec(s.into_bytes())),
                rep: InternalRep::None,
                rc: Cell::new(1),
            }))
    }

    /// Create a value from an integer
    pub fn from_int(n: i64) -> Self {
        // Immediate for the whole i63 range — clone/drop/compare without
        // the allocator or the refcount.  The extremes keep the heap
        // fallback (full i64 range preserved); the lazy-string rule is
        // tclsh's (an int renders never unless demanded).
        if ((i64::MIN >> 1)..=(i64::MAX >> 1)).contains(&n) {
            return Value::from_imm(n);
        }
        Value::from_vptr(VPtr::new(ValueInner {
            string: StringRep::new(),
            rep: InternalRep::Int(n),
            rc: Cell::new(1),
        }))
    }

    /// Create a value from an arbitrary-precision integer (D5).
    ///
    /// 三态下沉：i63 内落立即数（快路径零分配），i64 内落堆 Int 箱
    /// （既有语义不动——`as_int` 等全兼容），超界才落 `Big(Rc<BigInt>)`
    /// 箱。字符串形态惰性：首次 `as_str` 渲染 canonical 十进制。
    pub fn from_bigint(b: BigInt) -> Self {
        if let Some(n) = b.to_i64() {
            return Value::from_int(n);
        }
        Value::from_vptr(VPtr::new(ValueInner {
            string: StringRep::new(),
            rep: InternalRep::Big(Rc::new(b)),
            rc: Cell::new(1),
        }))
    }

    /// Exact integer view of a `Big`-rep value, `None` for every other
    /// state（含立即数/i64 堆箱——`as_int`/`bignum::int_rep` 才是全谱
    /// 入口）。借用形态：比较族免去弯路克隆。
    #[inline]
    pub fn as_big(&self) -> Option<&BigInt> {
        if self.is_imm() {
            return None;
        }
        match &unsafe { self.heap_inner() }.rep {
            InternalRep::Big(b) => Some(b),
            _ => None,
        }
    }

    /// Create a value from a float
    pub fn from_float(n: f64) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
                string: StringRep::new(),
                rep: InternalRep::Float(n),
                rc: Cell::new(1),
            }))
    }

    /// Create a value from a boolean
    pub fn from_bool(b: bool) -> Self {
        if b {
            CACHED_BOOL_TRUE.with(Value::from_heap_clone)
        } else {
            CACHED_BOOL_FALSE.with(Value::from_heap_clone)
        }
    }

    /// Create a value directly from a cached list of values.
    ///
    /// The string representation is lazily generated on first `as_str()`
    /// call — this avoids the serialize cost when the list is only ever
    /// accessed structurally (e.g. `lindex`, `lappend`).
    pub fn from_list_cached(items: Vec<Value>) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
            string: StringRep::new(), // lazy — generated on first as_str()
            rep: InternalRep::List(items),
            rc: Cell::new(1),
        }))
    }

    /// Create a value directly from a cached dict.
    ///
    /// The string representation is lazily generated on first `as_str()`.
    pub fn from_dict_cached(entries: DictMap) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
                string: StringRep::new(),
                rep: InternalRep::Dict(entries),
                rc: Cell::new(1),
            }))
    }

    /// Create a dict value from key-value pairs (ordered).
    pub fn from_dict_pairs(pairs: &[(Value, Value)]) -> Self {
        let mut map = FastIndexMap::with_capacity_and_hasher(pairs.len(), FastBuild::default());
        for (k, v) in pairs {
            map.insert(k.as_str().to_string(), v.clone());
        }
        Value::from_vptr(VPtr::new(ValueInner {
                string: StringRep::new(),
                rep: InternalRep::Dict(DictMap::Ordered(map)),
                rc: Cell::new(1),
            }))
    }

    /// Create a value from a list of values.
    ///
    /// The string form is generated lazily on first `as_str()` — eagerly
    /// materializing it here retains every parent's serialization forever
    /// in deeply nested lists (`set x [list $x {}]` grew to 24 GB RSS
    /// because each level kept its own full-length string; tclsh keeps
    /// the object tree structural and never stringifies, obj-32.1).
    pub fn from_list(items: &[Value]) -> Self {
        Self::from_list_cached(items.to_vec())
    }

    // ── lazy-string（§6）：宿主字符串池三态 ─────────────────────

    /// JsOnly 构造（lazy-string §6）：包一个宿主字符串池句柄为值。
    ///
    /// 字节 cell 保持空——首次字节级语义才经 `as_str` choke point 物化
    /// （JsOnly → Dual）。`idx` 必须是**本调用新近取得的所有权**（backing
    /// 的 `insert`/`concat` 返回值，或显式 `acquire` 后移交）：构造以 1 池
    /// 引用起算。无背板注册时宿主侧根本拿不到 idx，此构造器不会被触达。
    pub fn from_host_str(idx: u32) -> Self {
        Value::from_vptr(VPtr::new(ValueInner {
            string: StringRep::new(), // JsOnly：字节未物化
            rep: InternalRep::HostStr(idx),
            rc: Cell::new(1),
        }))
    }

    /// 读取宿主池句柄（不物化、不建池引用）。
    ///
    /// JsOnly/Dual 态返回 `Some(idx)`；其余 rep 返回 `None`。白名单操作
    /// （equal/concat/回传 JS/puts）与 rtcl-wasm 边界的零转码取串都用它
    /// 探测。
    #[inline]
    pub fn host_str_handle(&self) -> Option<u32> {
        if self.is_imm() {
            return None;
        }
        match unsafe { self.heap_inner() }.rep {
            InternalRep::HostStr(idx) => Some(idx),
            _ => None,
        }
    }

    /// Tcl→JS 反向穿越（NativeOnly ──首次──→ Dual）：确保值持有宿主池
    /// 句柄并返回它。
    ///
    /// - 已是 JsOnly/Dual：原句柄直返（二次穿越零成本）。
    /// - NativeOnly 且背板已注册：先经 `as_str` 物化字节（Dual 保留字节，
    ///   解释器侧不再重渲染），再 `insert` 入池、rep 指向槽位。
    /// - 无背板：`None`（原生宿主没有 JS 形态可言）。
    ///
    /// COW 安全：`heap_make_mut` 的深拷贝会克隆字节 cell；此处旧 rep 恒非
    /// HostStr（上面已早退），不触碰池引用计数。
    pub fn to_host_str(&mut self) -> Option<u32> {
        if let Some(idx) = self.host_str_handle() {
            return Some(idx);
        }
        let bytes = self.as_str();
        let idx = crate::host_string::with_backing(|b| b.insert(bytes.as_bytes()))?;
        let inner = self.heap_make_mut();
        inner.rep = InternalRep::HostStr(idx);
        Some(idx)
    }

    // ── Accessors ──────────────────────────────────────────────

    /// Get the string representation.
    ///
    /// If the string has not been materialized yet (lazy value), it is
    /// auto-generated from the internal representation via `OnceCell`.
    pub fn as_str(&self) -> &str {
        if self.is_imm() {
            // Materialize: upgrade to a heap box (one-way, idempotent)
            // and render into it — the returned &str borrows `self`,
            // whose box now holds the rendering.
            let n = self.imm();
            let fresh = VPtr::new(ValueInner {
                string: StringRep::with_bytes(SmallVec::from_slice(format_int(n).as_bytes())),
                rep: InternalRep::Int(n),
                rc: Cell::new(1),
            });
            self.bits.set(fresh.0.as_ptr() as u64);
            std::mem::forget(fresh); // rc 1 transfers into `bits`
        }
        let inner = unsafe { self.heap_inner() };
        let bytes = inner.string.bytes.get_or_init(|| {
            match &inner.rep {
                InternalRep::List(items) => {
                    materialize_below(items.iter());
                    SmallVec::from_slice(serialize_list(items).as_bytes())
                }
                InternalRep::Dict(map) => {
                    materialize_below(map.values());
                    SmallVec::from_slice(serialize_dict(map).as_bytes())
                }
                InternalRep::Int(n) => SmallVec::from_slice(format_int(*n).as_bytes()),
                // D5：bignum 的 canonical 十进制（num-bigint 的 to_string
                // 与 tclsh/libtommath 的十进制渲染一致，bnprobe2 钉形）。
                InternalRep::Big(b) => SmallVec::from_slice(b.to_string().as_bytes()),
                InternalRep::Float(n) => SmallVec::from_slice(format_float(*n).as_bytes()),
                InternalRep::Bool(b) => {
                    SmallVec::from_slice(if *b { b"1" } else { b"0" })
                }
                // lazy-string choke point（§6）：JsOnly 态的宿主串在此一处
                // 物化（JsOnly ──首次字节操作──→ Dual）。强制物化清单
                // （list/dict parse、string map/match/…、regexp、expr、
                // binary、eval）无需显式代码——它们全经 as_str 取字节。
                // 背板缺位（理论不可达：HostStr 只在有背板时构造）兜底空
                // 串，绝不 panic。
                InternalRep::HostStr(idx) => {
                    crate::host_string::with_backing(|b| b.to_bytes(*idx))
                        .unwrap_or_default()
                }
                InternalRep::None => SmallVec::new(),
            }
        });
        // Safety: we always store valid UTF-8
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    /// The string's length in characters (UTF-8 scalar values) — the D4
    /// cached read.
    ///
    /// 命中 O(1)；未命中按 [`char_len_of`]（T7 SWAR）重算一次并缓存，
    /// 计数落在本箱 `StringRep`（与字节同生命周期）。语义与既有口径
    /// 逐位同值（`chars().count()` ≡ 字节数 − SWAR 连续字节数，合法
    /// UTF-8 双射）；等价红线由 debug 构建的逐调用自检兜底（见
    /// `d4_selfcheck`），探针与 fuzz 对 oracle 再对一层。
    ///
    /// 共享箱写缓存（`Cell`）的安全性：字节受 COW 保护，缓存是字节的
    /// 纯函数——见 [`StringRep`] 文档。
    pub fn char_len(&self) -> usize {
        if self.is_imm() {
            // 立即数渲染恒为 ASCII 十进制——字节长即字符长（顺带完成
            // imm → 堆箱的一次性升级，与 as_str 同一状态机）。
            return self.as_str().len();
        }
        if self.is_unset() {
            return 0;
        }
        let cached = unsafe { self.heap_inner() }.string.char_len.get();
        if cached != CHAR_LEN_UNKNOWN {
            d4_selfcheck(self, cached);
            return cached;
        }
        let n = char_len_of(self.as_str());
        unsafe { self.heap_inner() }.string.char_len.set(n);
        d4_selfcheck(self, n);
        n
    }

    /// Append `text` to this value's string in place, and drop nothing
    /// else: `false` when the value is anything but a plain string (an
    /// integer, a list, …) or an immediate, and the caller must build the
    /// new string some other way.  A value shared with other owners is
    /// copied first, so the append never shows through to them (tclsh:
    /// `Tcl_AppendToObj` on the variable's own object, a duplicate when
    /// it is shared).  Appending in place is what keeps a loop of appends
    /// linear: the variable's buffer grows instead of being rebuilt from
    /// both halves on every call.
    ///
    /// D4：这是全仓唯一的**原地**字节改写入口（改写入口清点，见
    /// StringRep 文档）——char_len 已知时增量维护（O(len(text))，与
    /// extend 同阶；追加文本本就要过一遍），未知则保持惰性，不强算。
    pub(crate) fn append_str_in_place(&mut self, text: &str) -> bool {
        if self.is_imm() {
            return false;
        }
        let inner = self.heap_make_mut();
        if !matches!(inner.rep, InternalRep::None) {
            return false;
        }
        // Materialised first: an empty string nobody has asked the bytes
        // of has no buffer yet.
        let _ = inner.string.bytes.get_or_init(SmallVec::new);
        if let Some(bytes) = inner.string.bytes.get_mut() {
            bytes.extend_from_slice(text.as_bytes());
            let cl = inner.string.char_len.get();
            if cl != CHAR_LEN_UNKNOWN {
                inner.string.char_len.set(cl + char_len_of(text));
            }
        }
        true
    }

    /// Ensure the string representation is materialized.
    /// With `OnceCell`, `as_str()` auto-materializes, so this is now a convenience.
    pub fn ensure_string(&mut self) {
        let _ = self.as_str();
    }

    /// Get the string representation, materializing it if needed.
    /// Now simply delegates to `as_str()` since `OnceCell` handles lazy init.
    pub fn to_str(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.as_str())
    }

    /// Try to get as integer
    ///
    /// The Int/Bool arms are `#[inline]` (every compiled loop condition and
    /// `incr` lands here); the string-rep arm is outlined so call sites
    /// don't carry the radix parser's code.
    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        if self.is_imm() {
            return Some(self.imm());
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Int(n) => Some(*n),
            // D5：bignum 超出 i64 视为"非 i64 整数"——不物化字符串、不
            // 走十进制重解析（精确视图走 `bignum::int_rep`/`as_big`）。
            InternalRep::Big(_) => None,
            // The cached bool rep's string is exactly "1"/"0" — the fast
            // path every loop condition takes (Tcl_GetBooleanFromObj's
            // numeric arm), not a from-scratch decimal parse.
            InternalRep::Bool(b) => Some(*b as i64),
            _ => self.as_int_from_string(),
        }
    }

    /// The string-rep arm of [`Self::as_int`], outlined.
    #[inline(never)]
    fn as_int_from_string(&self) -> Option<i64> {
        // One grammar for every caller (types::bignum::parse_int): an
        // i64-range literal here, `None` for bignums (the wide re-parse is
        // the caller's) and for everything that is not an integer.
        match crate::types::bignum::parse_int(self.to_str().trim())? {
            crate::types::bignum::IntRep::I64(i) => Some(i),
            crate::types::bignum::IntRep::Big(_) => None,
        }
    }

    /// Try to get as float (numeric arms `#[inline]`, string arm outlined —
    /// see [`Self::as_int`]).
    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        if self.is_imm() {
            return Some(self.imm() as f64);
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Float(n) => Some(*n),
            InternalRep::Int(n) => Some(*n as f64),
            // D5：bignum 的最近双精度值（round-to-even，tclsh 的
            // bignum→double 路径同语义；超界给 ±Inf）。
            InternalRep::Big(b) => Some(crate::types::bignum::to_f64(b)),
            InternalRep::Bool(b) => Some(u8::from(*b) as f64),
            _ => self.as_float_from_string(),
        }
    }

    /// The string-rep arm of [`Self::as_float`], outlined.
    #[inline(never)]
    fn as_float_from_string(&self) -> Option<f64> {
        let s = self.to_str();
        let s = s.trim();
        // Tcl accepts Inf / -Inf / NaN (any case) as numeric strings.
        match s.to_ascii_lowercase().as_str() {
            "inf" | "infinity" => return Some(f64::INFINITY),
            "-inf" | "-infinity" => return Some(f64::NEG_INFINITY),
            "nan" => return Some(f64::NAN),
            _ => {}
        }
        // `018` is an invalid octal number, not the double 18.0.
        if crate::types::bignum::is_invalid_octal(s) {
            return None;
        }
        // A wide integer literal is its exact value as a double: the
        // digits of `0o777…` are not the decimal they look like.
        if let Some(crate::types::bignum::IntRep::Big(b)) = crate::types::bignum::parse_int(s) {
            return num_traits::ToPrimitive::to_f64(&b);
        }
        f64::from_str(s).ok()
    }

    /// Float-rep-only read（C-float 守卫专用，§5「类型分派先于 opcode」）：
    /// 只有缓存 rep 恰为 Float 才给值——字符串/整值/布尔一律 None，不
    /// shimmer。JIT 的 `unbox_float` 守卫据此分派：Float rep 走 f64 内
    /// 联快车道；`"1.5"`、整数、bignum 的全语义裁决留给 binary_op 宿主
    /// 兜底（`"2"*"3"` 产 6 而非 6.0 的分界面就在这里）。
    pub fn as_float_rep(&self) -> Option<f64> {
        if self.is_imm() {
            return None;
        }
        match &unsafe { self.heap_inner() }.rep {
            InternalRep::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to get as boolean
    ///
    /// Tcl rules (TclGetBooleanFromObj): any numeric value coerces by
    /// zero-ness (2.5 → true, 0x10 → true, 00 → false); otherwise the
    /// string must be an unambiguous prefix of true/false/yes/no/on/off
    /// (`t`, `ye`, `of` are valid; `o` alone is ambiguous → None).
    pub fn as_bool(&self) -> Option<bool> {
        if self.is_imm() {
            return Some(self.imm() != 0);
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Bool(b) => Some(*b),
            _ => {
                if let Some(i) = self.as_int() {
                    return Some(i != 0);
                }
                if let Some(f) = self.as_float() {
                    return if f.is_nan() { None } else { Some(f != 0.0) };
                }
                let s = self.to_str();
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                let lower = s.to_ascii_lowercase();
                let mut candidates = ["true", "false", "yes", "no", "on", "off"]
                    .iter()
                    .filter(|word| word.starts_with(lower.as_str()));
                let first = *candidates.next()?;
                if candidates.next().is_some() {
                    return None; // ambiguous prefix ("o")
                }
                Some(matches!(first, "true" | "yes" | "on"))
            }
        }
    }

    /// Get the cached list if available, or parse from string.
    ///
    /// Returns a freshly parsed `Vec<Value>` — callers that need to
    /// mutate should use the `_mut` helpers instead.
    pub fn as_list(&self) -> Option<Vec<Value>> {
        if self.is_imm() {
            // An immediate's rendering is its list form ("5" -> {5}) —
            // the same string-parse the old Int-rep `_` arm took.
            let s = self.as_str();
            return parse_list(s);
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::List(items) => Some(items.clone()),
            InternalRep::Dict(map) => {
                Some(map.iter().flat_map(|(k, v)| [Value::from_str(k), v.clone()]).collect())
            }
            _ => {
                let s = self.to_str();
                parse_list(&s)
            }
        }
    }

    /// Like `as_list()`, but malformed list strings yield Tcl's parse
    /// error (message + `::errorCode`) instead of `None`.
    pub fn as_list_strict(&self) -> std::result::Result<Vec<Value>, ListParseError> {
        if self.is_imm() {
            let s = self.as_str();
            return parse_list_full(s);
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::List(items) => Ok(items.clone()),
            InternalRep::Dict(map) => {
                Ok(map.iter().flat_map(|(k, v)| [Value::from_str(k), v.clone()]).collect())
            }
            _ => {
                let s = self.to_str();
                parse_list_full(&s)
            }
        }
    }

    /// Get a reference to the dict's DictMap if the internal rep is Dict.
    /// Zero-copy — does not clone.
    pub fn as_dict_ref(&self) -> Option<&DictMap> {
        if self.is_imm() {
            return None;
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Dict(map) => Some(map),
            _ => None,
        }
    }

    /// Get a mutable reference to the dict's DictMap (COW).
    pub fn as_dict_mut(&mut self) -> Option<&mut DictMap> {
        if self.is_imm() {
            return None;
        }
        if !matches!(unsafe { self.heap_inner() }.rep, InternalRep::Dict(_)) {
            return None;
        }
        let inner = self.heap_make_mut();
        inner.string = StringRep::new(); // invalidate string（D4：计数随字节一起作废）
        match &mut inner.rep {
            InternalRep::Dict(map) => Some(map),
            _ => None,
        }
    }

    /// Get a mutable reference to the list items (COW) — the
    /// `as_dict_mut` mirror for lists.  In-place when this handle is
    /// the value's sole owner (the interpreter's mutation fast path
    /// takes the variable out of its slot first, tclsh's refcnt==1
    /// mutation); otherwise the `ValueInner` is cloned once.
    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Value>> {
        if self.is_imm() {
            return None;
        }
        if !matches!(unsafe { self.heap_inner() }.rep, InternalRep::List(_)) {
            return None;
        }
        let inner = self.heap_make_mut();
        inner.string = StringRep::new(); // invalidate string（D4：计数随字节一起作废）
        match &mut inner.rep {
            InternalRep::List(items) => Some(items),
            _ => None,
        }
    }

    /// Overwrite the integer rep in place (COW) — the `incr` fast path,
    /// tclsh's TclIncrObj on a refcnt==1 object: no allocation, and the
    /// cached string rendering is dropped instead of eagerly rebuilt.
    pub fn set_int_rep(&mut self, n: i64) {
        if self.is_imm() {
            // Immediate ints retag in place — no allocation, no refcount.
            if ((i64::MIN >> 1)..=(i64::MAX >> 1)).contains(&n) {
                self.set_imm(n);
                return;
            }
            // 超出 i63 imm 域的结果（如 2^62-1 + 5）没有堆箱可复用：
            // 整值换新（from_int 落回堆装箱）。不能落入 heap_make_mut——
            // 那会把 imm 位型当 *mut ValueInner 解引用（f004649 的
            // 未对齐崩溃根因）。
            *self = Value::from_int(n);
            return;
        }
        let inner = self.heap_make_mut();
        inner.rep = InternalRep::Int(n);
        inner.string = StringRep::new(); // invalidate string（D4：计数随字节一起作废）
    }

    /// Do both handles point at the same `ValueInner` allocation?
    ///
    /// Pointer identity, not string equality — the loop-body memo keys on
    /// "is this the exact script Value the interpreter handed out last
    /// time".  Clones share the allocation and still compare equal; a
    /// value rebuilt from text compares different even when the text
    /// matches, so a stale memo can never be hit through a look-alike.
    pub fn same_allocation(&self, other: &Value) -> bool {
        self.bits.get() == other.bits.get()
    }

    /// Parse or return a dict as an owned DictMap.
    pub fn as_dict(&self) -> Option<DictMap> {
        if self.is_imm() {
            let s = self.as_str();
            let list = parse_list(s)?;
            if list.len() % 2 != 0 {
                return None;
            }
            let mut map = DictMap::ordered_with_capacity(list.len() / 2);
            for c in list.chunks(2) {
                map.insert(c[0].as_str().to_string(), c[1].clone());
            }
            return Some(map);
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Dict(map) => Some(map.clone()),
            InternalRep::List(items) => {
                if items.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(items.len() / 2);
                for c in items.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(map)
            }
            _ => {
                let s = self.to_str();
                let list = parse_list(&s)?;
                if list.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(list.len() / 2);
                for c in list.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(map)
            }
        }
    }

    /// Borrow the dict without cloning when the internal rep is already Dict.
    /// Returns `Cow::Borrowed` for zero-copy access, `Cow::Owned` when parsing is needed.
    pub fn as_dict_cow(&self) -> Option<std::borrow::Cow<'_, DictMap>> {
        if self.is_imm() {
            return None;
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::Dict(map) => Some(std::borrow::Cow::Borrowed(map)),
            InternalRep::List(items) => {
                if items.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(items.len() / 2);
                for c in items.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(std::borrow::Cow::Owned(map))
            }
            _ => {
                let s = self.to_str();
                let list = parse_list(&s)?;
                if list.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(list.len() / 2);
                for c in list.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(std::borrow::Cow::Owned(map))
            }
        }
    }

    /// Check if the value is empty
    pub fn is_empty(&self) -> bool {
        if self.is_imm() {
            return false;
        }
        let inner = unsafe { self.heap_inner() };
        if let Some(bytes) = inner.string.bytes.get() {
            bytes.is_empty()
        } else {
            match &inner.rep {
                InternalRep::List(items) => items.is_empty(),
                InternalRep::Dict(map) => map.is_empty(),
                InternalRep::None => true,
                // JsOnly 态：免物化问池（合法串的字节空 ⇔ 宿主形态空）。
                // `if $x` 等 is_true 兜底路径依赖空串为假。
                InternalRep::HostStr(idx) => crate::host_string::with_backing(
                    |b| b.is_empty(*idx),
                )
                .unwrap_or(true),
                _ => false,
            }
        }
    }

    /// Get the length of the string representation
    pub fn len(&self) -> usize {
        let s = self.to_str();
        s.len()
    }

    /// Check if the value is a valid number
    pub fn is_number(&self) -> bool {
        self.as_int().is_some() || self.as_float().is_some()
    }

    /// Concatenate two values as strings
    pub fn concat(&self, other: &Value) -> Value {
        let a = self.to_str();
        let b = other.to_str();
        let mut result = String::with_capacity(a.len() + b.len());
        result.push_str(&a);
        result.push_str(&b);
        Value::from_str(&result)
    }

    /// Compare two values
    pub fn compare(&self, other: &Value) -> core::cmp::Ordering {
        let a = self.to_str();
        let b = other.to_str();
        a.cmp(&b)
    }

    /// Compare two values numerically if possible
    pub fn compare_numeric(&self, other: &Value) -> Option<core::cmp::Ordering> {
        match (self.as_float(), other.as_float()) {
            (Some(a), Some(b)) => Some(a.partial_cmp(&b)?),
            _ => None,
        }
    }

    /// Check if the value is true (for conditionals)
    pub fn is_true(&self) -> bool {
        self.as_bool().unwrap_or_else(|| !self.is_empty())
    }

    // ── Type introspection (mirrors jimtcl's typePtr) ──────────

    /// Return the name of the current internal representation type.
    ///
    /// Mirrors jimtcl's `typePtr->name` — answers "what type **is** this
    /// value right now?" without triggering any conversion.
    ///
    /// Possible return values: `"string"`, `"int"`, `"float"`, `"bool"`,
    /// `"list"`, `"dict"`.
    pub fn type_name(&self) -> &'static str {
        if self.is_imm() {
            return "int";
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::None => "string",
            // 宿主串就是 string rep 的宿主原生形态——对外类型名不变。
            InternalRep::HostStr(_) => "string",
            InternalRep::Int(_) | InternalRep::Big(_) => "int",
            InternalRep::Float(_) => "float",
            InternalRep::Bool(_) => "bool",
            InternalRep::List(_) => "list",
            InternalRep::Dict(_) => "dict",
        }
    }

    /// Borrow the list **only** if the internal rep is already `List`.
    ///
    /// Unlike `as_list()` this never parses a string into a list,
    /// mirroring the semantics of `as_dict_ref()`.
    pub fn as_list_ref(&self) -> Option<&[Value]> {
        if self.is_imm() {
            return None;
        }
        let inner = unsafe { self.heap_inner() };
        match &inner.rep {
            InternalRep::List(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    // ── Rc / COW helpers ───────────────────────────────────────

    /// Returns `true` if this value is shared (reference count > 1).
    ///
    /// Equivalent to jimtcl's `Jim_IsShared()`.
    pub fn is_shared(&self) -> bool {
        !self.is_imm() && unsafe { self.heap_inner().rc.get() > 1 }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.to_str();
        write!(f, "{}", s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::from_str(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::from_string(s)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::from_int(n)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::from_int(n as i64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::from_bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::from_float(n)
    }
}

/// Serialize dict entries into a Tcl list string (key1 val1 key2 val2 ...).
/// Materialize the string of every list/dict value below `children`,
/// post-order, with an explicit stack, before the caller renders its own
/// level.
///
/// `serialize_list` calls `as_str` on each element, so an unmaterialized
/// nested list made that call recurse once per nesting level: about
/// 14,000 levels of `set l [list $l]` followed by `string length $l` ran
/// the native stack out (a wasm instance traps at ~5,000). When this
/// returns, every descendant's string cell is set and the caller's own
/// `as_str` renders exactly one level. Each level is visited once, the
/// same work the recursive form did.
fn materialize_below<'a>(children: impl Iterator<Item = &'a Value>) {
    let mut stack: Vec<Frame<'a>> = Vec::new();
    for child in children {
        if let Some(frame) = pending_frame(child) {
            stack.push(frame);
        }
    }
    while let Some(frame) = stack.last_mut() {
        let child = match frame.kids.next() {
            Some(child) => child,
            None => {
                // Its children are done: this renders one level.
                let done = stack.pop().expect("frame");
                let _ = done.owner.as_str();
                continue;
            }
        };
        if let Some(frame) = pending_frame(child) {
            stack.push(frame);
        }
    }
}

/// A value under materialization plus the iterator over its children.
struct Frame<'a> {
    owner: &'a Value,
    kids: Kids<'a>,
}

enum Kids<'a> {
    List(std::slice::Iter<'a, Value>),
    Dict(DictValues<'a>),
}

impl<'a> Iterator for Kids<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<&'a Value> {
        match self {
            Kids::List(i) => i.next(),
            Kids::Dict(i) => i.next(),
        }
    }
}

/// A list/dict value whose string is not built yet (it needs its children
/// first); anything else needs no frame.
fn pending_frame(v: &Value) -> Option<Frame<'_>> {
    if v.is_imm() || v.is_unset() {
        return None;
    }
    let inner = unsafe { v.heap_inner() };
    if inner.string.bytes.get().is_some() {
        return None;
    }
    match &inner.rep {
        InternalRep::List(items) => Some(Frame { owner: v, kids: Kids::List(items.iter()) }),
        InternalRep::Dict(map) => Some(Frame { owner: v, kids: Kids::Dict(map.values()) }),
        _ => None,
    }
}

fn serialize_dict(map: &DictMap) -> String {
    let items: Vec<Value> = map
        .iter()
        .flat_map(|(k, v)| [Value::from_str(k), v.clone()])
        .collect();
    serialize_list(&items)
}

/// Serialize a slice of values into a Tcl list string.
///
/// This is a faithful port of Tcl 8.6's `TclScanElement` /
/// `TclConvertElement` / `Tcl_Merge` (generic/tclUtil.c, COMPAT=1): each
/// element is emitted verbatim, brace-quoted, or backslash-escaped
/// according to its contents; elements are joined with single spaces; a
/// leading `#` is quoted only for the first element.
fn serialize_list(items: &[Value]) -> String {
    let mut out: Vec<u8> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        convert_element(&mut out, item.as_str().as_bytes(), i == 0);
    }
    // Safe: item strings are valid UTF-8, we only insert ASCII bytes and
    // never split a multi-byte sequence.
    String::from_utf8(out).unwrap_or_default()
}

/// Tcl's list whitespace: the only bytes that separate list elements.
pub fn is_tcl_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// How a list element must be quoted (mirrors Tcl's CONVERT_* flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvMode {
    /// No quoting needed.
    None,
    /// Enclose in braces.
    Brace,
    /// Backslash-escape every special character (including braces).
    Escape,
    /// Backslash-escape specials except braces (historical Tcl mode used
    /// when quoting is needed solely due to `]` or `"`).
    Mask,
}

/// Port of Tcl's `TclScanElement`: decide how `s` must be quoted.
fn scan_element(s: &[u8]) -> ConvMode {
    if s.is_empty() {
        return ConvMode::Brace;
    }
    let mut forbid_none = false;
    let mut require_escape = false;
    let mut prefer_escape = false;
    let mut prefer_brace = false;
    let mut nesting: i64 = 0;

    if s[0] == b'{' || s[0] == b'"' {
        // Leading character would be misread as list syntax.
        forbid_none = true;
        prefer_brace = true;
    }

    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'{' => nesting += 1,
            b'}' => {
                nesting -= 1;
                if nesting < 0 {
                    require_escape = true;
                }
            }
            b']' | b'"' => {
                forbid_none = true;
                prefer_escape = true;
            }
            b'[' | b'$' | b';' => {
                forbid_none = true;
                prefer_brace = true;
            }
            b'\\' => {
                if i + 1 == s.len() {
                    // Final backslash cannot be brace-quoted.
                    require_escape = true;
                } else if s[i + 1] == b'\n' {
                    // Backslash-newline cannot be brace-quoted.
                    require_escape = true;
                    i += 1;
                } else if matches!(s[i + 1], b'{' | b'}' | b'\\') {
                    i += 1;
                }
                forbid_none = true;
                prefer_brace = true;
            }
            c if is_tcl_space(c) => {
                forbid_none = true;
                prefer_brace = true;
            }
            _ => {}
        }
        i += 1;
    }

    if nesting != 0 {
        require_escape = true;
    }
    if require_escape {
        ConvMode::Escape
    } else if forbid_none {
        if prefer_escape && !prefer_brace {
            ConvMode::Mask
        } else {
            ConvMode::Brace
        }
    } else {
        ConvMode::None
    }
}

/// Port of Tcl's `TclConvertElement`: append the list representation of
/// `s` to `out`. `quote_hash` mirrors `!TCL_DONT_QUOTE_HASH` (only true
/// for the first element of a list).
fn convert_element(out: &mut Vec<u8>, s: &[u8], quote_hash: bool) {
    if s.is_empty() {
        out.extend_from_slice(b"{}");
        return;
    }
    let mut mode = scan_element(s);
    let mut start = 0;
    if quote_hash && s[0] == b'#' {
        if mode == ConvMode::Escape {
            out.extend_from_slice(b"\\#");
            start = 1;
        } else {
            mode = ConvMode::Brace;
        }
    }
    match mode {
        ConvMode::None => out.extend_from_slice(&s[start..]),
        ConvMode::Brace => {
            out.push(b'{');
            out.extend_from_slice(&s[start..]);
            out.push(b'}');
        }
        ConvMode::Escape | ConvMode::Mask => {
            for &b in &s[start..] {
                match b {
                    b']' | b'[' | b'$' | b';' | b' ' | b'\\' | b'"' => {
                        out.push(b'\\');
                        out.push(b);
                    }
                    b'{' | b'}' => {
                        if mode == ConvMode::Escape {
                            out.push(b'\\');
                        }
                        out.push(b);
                    }
                    0x0c => out.extend_from_slice(b"\\f"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    0x0b => out.extend_from_slice(b"\\v"),
                    _ => out.push(b),
                }
            }
        }
    }
}

/// Check if a string needs braces for list representation
fn needs_braces(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let mut brace_depth = 0i32;
    for c in s.chars() {
        match c {
            '{' => brace_depth += 1,
            '}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return true;
                }
            }
            ' ' | '\t' | '\n' | '\r' | ';' | '"' | '\\' | '[' | ']' | '$' => {
                return true;
            }
            _ => {}
        }
    }
    brace_depth != 0
}

/// Parse a string as a Tcl list. Returns `None` for malformed lists
/// (unbalanced braces/quotes, junk after a braced or quoted element).
fn parse_list(s: &str) -> Option<Vec<Value>> {
    parse_list_full(s).ok()
}

/// Error produced when parsing a malformed Tcl list string.
///
/// `message` matches Tcl 8.6's error text (e.g. "unmatched open brace in
/// list"); `code` is the corresponding `::errorCode` value
/// (e.g. "TCL VALUE LIST BRACE").
#[derive(Debug, Clone)]
pub struct ListParseError {
    pub message: String,
    pub code: &'static str,
}

/// Parse a string as a Tcl list, reporting Tcl's error on malformed input.
///
/// This is a port of Tcl 8.6's `FindElement` (generic/tclUtil.c): elements
/// are separated by ASCII whitespace; braced elements keep their contents
/// verbatim (backslashes only affect brace counting); quoted and bare
/// elements undergo backslash substitution (`TclCopyAndCollapse`).
pub fn parse_list_full(s: &str) -> std::result::Result<Vec<Value>, ListParseError> {
    parse_elements(s, false)
}

/// Parse a string as a Tcl dictionary's element sequence, reporting Tcl's
/// dict-specific error texts and errorCodes. `tclDictObj.c` shares the
/// list element scanner but says "dict element in braces followed by ..."
/// and raises TCL VALUE DICTIONARY JUNK/BRACE/QUOTE.
pub fn parse_dict_full(s: &str) -> std::result::Result<Vec<Value>, ListParseError> {
    parse_elements(s, true)
}

fn parse_elements(s: &str, dict: bool) -> std::result::Result<Vec<Value>, ListParseError> {
    let b = s.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && is_tcl_space(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let (elem, next) = find_element(s, i, dict)?;
        result.push(Value::from_str(&elem));
        i = next;
    }
    Ok(result)
}

fn junk_error(s: &str, i: usize, kind: &str, dict: bool) -> ListParseError {
    // Tcl reports up to 20 bytes of the offending text.
    let b = s.as_bytes();
    let mut j = i;
    while j < b.len() && !is_tcl_space(b[j]) && j < i + 20 {
        j += 1;
    }
    while !s.is_char_boundary(j) {
        j -= 1;
    }
    let (what, code) = if dict {
        ("dict", "TCL VALUE DICTIONARY JUNK")
    } else {
        ("list", "TCL VALUE LIST JUNK")
    };
    ListParseError {
        message: format!(
            "{} element in {} followed by \"{}\" instead of space",
            what,
            kind,
            &s[i..j]
        ),
        code,
    }
}

/// Locate the list element starting at byte `start` (which must not be
/// whitespace). Returns the element value and the position just past any
/// whitespace following the element.
fn find_element(
    s: &str,
    start: usize,
    dict: bool,
) -> std::result::Result<(String, usize), ListParseError> {
    let b = s.as_bytes();
    let mut i = start;

    if b[i] == b'{' {
        // Braced element: contents are literal; backslashes only matter
        // for brace counting.
        i += 1;
        let elem_start = i;
        let mut depth = 1i32;
        while i < b.len() {
            match b[i] {
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        if i >= b.len() || is_tcl_space(b[i]) {
                            let mut j = i;
                            while j < b.len() && is_tcl_space(b[j]) {
                                j += 1;
                            }
                            return Ok((s[elem_start..i - 1].to_string(), j));
                        }
                        return Err(junk_error(s, i, "braces", dict));
                    }
                }
                b'\\' => {
                    let (n, _) = scan_backslash(s, i);
                    i += n;
                }
                _ => i += 1,
            }
        }
        return Err(if dict {
            ListParseError {
                message: "unmatched open brace in dict".to_string(),
                code: "TCL VALUE DICTIONARY BRACE",
            }
        } else {
            ListParseError {
                message: "unmatched open brace in list".to_string(),
                code: "TCL VALUE LIST BRACE",
            }
        });
    }

    if b[i] == b'"' {
        // Quoted element: backslash substitution applies.
        i += 1;
        let elem_start = i;
        let mut has_backslash = false;
        while i < b.len() {
            match b[i] {
                b'"' => {
                    let raw = &s[elem_start..i];
                    i += 1;
                    if i >= b.len() || is_tcl_space(b[i]) {
                        let mut j = i;
                        while j < b.len() && is_tcl_space(b[j]) {
                            j += 1;
                        }
                        let v = if has_backslash { collapse(raw) } else { raw.to_string() };
                        return Ok((v, j));
                    }
                    return Err(junk_error(s, i, "quotes", dict));
                }
                b'\\' => {
                    has_backslash = true;
                    let (n, _) = scan_backslash(s, i);
                    i += n;
                }
                _ => i += 1,
            }
        }
        return Err(if dict {
            ListParseError {
                message: "unmatched open quote in dict".to_string(),
                code: "TCL VALUE DICTIONARY QUOTE",
            }
        } else {
            ListParseError {
                message: "unmatched open quote in list".to_string(),
                code: "TCL VALUE LIST QUOTE",
            }
        });
    }

    // Bare element: ends at whitespace; backslash substitution applies.
    let elem_start = i;
    let mut has_backslash = false;
    while i < b.len() && !is_tcl_space(b[i]) {
        if b[i] == b'\\' {
            has_backslash = true;
            let (n, _) = scan_backslash(s, i);
            i += n;
        } else {
            i += 1;
        }
    }
    let raw = &s[elem_start..i];
    let v = if has_backslash { collapse(raw) } else { raw.to_string() };
    Ok((v, i))
}

/// Substitute all backslash sequences in `s` (Tcl's `TclCopyAndCollapse`).
fn collapse(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let (n, rep) = scan_backslash(s, i);
            out.push_str(&rep);
            i += n;
        } else {
            let ch_len = utf8_len(b[i]);
            out.push_str(&s[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

/// Length in bytes of the UTF-8 sequence starting with byte `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >= 0xf0 {
        4
    } else if b >= 0xe0 {
        3
    } else {
        2
    }
}

/// Scan the backslash sequence starting at `s[i]` (which must be `\\`),
/// following Tcl's `TclParseBackslash` rules. Returns the number of bytes
/// consumed (including the backslash) and the substituted text.
fn scan_backslash(s: &str, i: usize) -> (usize, String) {
    let b = s.as_bytes();
    debug_assert!(b[i] == b'\\');
    if i + 1 >= b.len() {
        // Lone trailing backslash stays a backslash.
        return (1, "\\".to_string());
    }
    let c = b[i + 1];
    match c {
        b'a' => (2, "\x07".to_string()),
        b'b' => (2, "\x08".to_string()),
        b'f' => (2, "\x0c".to_string()),
        b'n' => (2, "\n".to_string()),
        b'r' => (2, "\r".to_string()),
        b't' => (2, "\t".to_string()),
        b'v' => (2, "\x0b".to_string()),
        b'x' => {
            let mut val: u32 = 0;
            let mut n = 0;
            while n < 2 && i + 2 + n < b.len() && (b[i + 2 + n] as char).is_ascii_hexdigit() {
                val = val * 16 + (b[i + 2 + n] as char).to_digit(16).unwrap_or(0);
                n += 1;
            }
            if n == 0 {
                (2, "x".to_string())
            } else {
                let ch = char::from_u32(val & 0xff).unwrap_or('\u{fffd}');
                (2 + n, ch.to_string())
            }
        }
        b'u' | b'U' => {
            let max_digits = if c == b'u' { 4 } else { 8 };
            let mut val: u32 = 0;
            let mut n = 0;
            while n < max_digits && i + 2 + n < b.len() && (b[i + 2 + n] as char).is_ascii_hexdigit()
            {
                val = val
                    .saturating_mul(16)
                    .saturating_add((b[i + 2 + n] as char).to_digit(16).unwrap_or(0));
                n += 1;
            }
            if n == 0 {
                return (2, (c as char).to_string());
            }
            // Combine a \u high surrogate with a following \u low surrogate.
            if c == b'u'
                && n == 4
                && (0xd800..0xdc00).contains(&val)
                && i + 2 + n + 1 < b.len()
                && b[i + 2 + n] == b'\\'
                && b[i + 2 + n + 1] == b'u'
            {
                let lo_start = i + 2 + n + 2;
                let mut low: u32 = 0;
                let mut m = 0;
                while m < 4 && lo_start + m < b.len() && (b[lo_start + m] as char).is_ascii_hexdigit()
                {
                    low = low * 16 + (b[lo_start + m] as char).to_digit(16).unwrap_or(0);
                    m += 1;
                }
                if m == 4 && (0xdc00..0xe000).contains(&low) {
                    let combined = ((val & 0x3ff) << 10 | (low & 0x3ff)) + 0x10000;
                    let ch = char::from_u32(combined).unwrap_or('\u{fffd}');
                    return (2 + n + 2 + m, ch.to_string());
                }
            }
            if (0xd800..0xe000).contains(&val) {
                return (2 + n, '\u{fffd}'.to_string());
            }
            let ch = char::from_u32(val).unwrap_or('\u{fffd}');
            (2 + n, ch.to_string())
        }
        b'0'..=b'7' => {
            let mut val: u32 = (c - b'0') as u32;
            let mut n = 1;
            while n < 3 && i + 1 + n < b.len() && matches!(b[i + 1 + n], b'0'..=b'7') {
                // Tcl only takes a third octal digit when the value stays < 256.
                if n == 2 && val >= 0x20 {
                    break;
                }
                val = val * 8 + (b[i + 1 + n] - b'0') as u32;
                n += 1;
            }
            let ch = char::from_u32(val & 0xff).unwrap_or('\u{fffd}');
            (1 + n, ch.to_string())
        }
        b'\n' => {
            // Backslash-newline and following spaces/tabs collapse to one space.
            let mut j = i + 2;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            (j - i, " ".to_string())
        }
        _ => {
            // Unknown escape: the backslash quotes the following character.
            let ch_len = utf8_len(c);
            let end = (i + 1 + ch_len).min(b.len());
            (1 + (end - (i + 1)), s[i + 1..end].to_string())
        }
    }
}

/// Format an integer for Tcl
fn format_int(n: i64) -> String {
    format!("{}", n)
}

/// Format a float for Tcl
fn format_float(n: f64) -> String {
    // tclsh renders non-finite floats as Inf / -Inf / NaN, not Rust's
    // lowercase "inf"/"NaN".
    if n.is_infinite() {
        return if n > 0.0 { "Inf".to_string() } else { "-Inf".to_string() };
    }
    if n.is_nan() {
        // 符号位跟随（remquo 域错误的 NaN 在 tclsh 印作 "-NaN"）。
        return if n.is_sign_negative() {
            "-NaN".to_string()
        } else {
            "NaN".to_string()
        };
    }
    // tclsh (8.6.17, verified): shortest round-trip digits, laid out in
    // fixed notation when the decimal exponent is in -4..=16, exponent
    // form otherwise ("1e+17", "1e-5" — exponent without leading zeros);
    // integral fixed values keep a trailing ".0".
    let e = format!("{:e}", n); // shortest repr, e.g. "-6.02e23"
    let (mant, exp_str) = e.split_once('e').unwrap();
    let exp: i32 = exp_str.parse().unwrap_or(0);
    let neg = mant.starts_with('-');
    let mant = mant.trim_start_matches('-');
    let mut digits: String = mant.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let mut body = String::new();
    if (-4..=16).contains(&exp) {
        let nd = digits.len() as i32;
        if exp >= nd - 1 {
            body.push_str(&digits);
            for _ in 0..(exp - (nd - 1)) {
                body.push('0');
            }
            body.push_str(".0");
        } else if exp >= 0 {
            body.push_str(&digits[..(exp + 1) as usize]);
            body.push('.');
            body.push_str(&digits[(exp + 1) as usize..]);
        } else {
            body.push_str("0.");
            for _ in 0..(-exp - 1) {
                body.push('0');
            }
            body.push_str(&digits);
        }
    } else {
        body.push_str(&digits[..1]);
        if digits.len() > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        body.push('e');
        if exp < 0 {
            body.push('-');
        } else {
            body.push('+');
        }
        body.push_str(&exp.abs().to_string());
    }
    if neg {
        body.insert(0, '-');
    }
    body
}

/// Quote a string for safe use as a Tcl word in a command.
/// Uses braces when the string contains special chars.
pub fn tcl_quote(s: &str) -> String {
    // Use the same scanner as list serialization: a blind `{...}` wrapper
    // produces an unparsable element when `s` itself has unbalanced braces
    // (e.g. an errorInfo excerpt quoting `set z {`).  Tcl falls back to
    // backslash escaping in that case.
    let mut out: Vec<u8> = Vec::with_capacity(s.len() + 2);
    convert_element(&mut out, s.as_bytes(), true);
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_from_str() {
        let v = Value::from_str("hello");
        assert_eq!(v.as_str(), "hello");
    }

    #[test]
    fn test_value_from_int() {
        let v = Value::from_int(42);
        assert_eq!(v.as_str(), "42");
        assert_eq!(v.as_int(), Some(42));
    }

    #[test]
    fn test_value_from_bool() {
        let v = Value::from_bool(true);
        assert_eq!(v.as_str(), "1");
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn test_list_parsing() {
        let v = Value::from_str("a b c");
        let list = v.as_list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].as_str(), "a");
        assert_eq!(list[1].as_str(), "b");
        assert_eq!(list[2].as_str(), "c");
    }

    #[test]
    fn test_braced_list() {
        let v = Value::from_str("{a b} {c d}");
        let list = v.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_str(), "a b");
        assert_eq!(list[1].as_str(), "c d");
    }

    #[test]
    fn test_serialize_matches_tcl() {
        // Oracle outputs from tclsh 8.6.17 `list`.
        let cases: &[(&[&str], &str)] = &[
            (&["a}b"], "a\\}b"),
            (&["a{b"], "a\\{b"),
            (&["a{b}c"], "a{b}c"),
            (&["a{b}c{d"], "a\\{b\\}c\\{d"),
            (&["a b"], "{a b}"),
            (&["a\nb"], "{a\nb}"),
            (&[""], "{}"),
            (&["\\"], "\\\\"),
            (&["\\n"], "{\\n}"),
            (&["a\"b"], "a\\\"b"),
            (&["a]b"], "a\\]b"),
            (&["a[b"], "{a[b}"),
            (&["a$b"], "{a$b}"),
            (&["a;b"], "{a;b}"),
            (&["{"], "\\{"),
            (&["}"], "\\}"),
            (&["#foo"], "{#foo}"),
            (&["a", "#foo"], "a #foo"),
            (&["#", "#"], "{#} #"),
            (&["#{"], "\\#\\{"),
            (&["a", "#{"], "a #\\{"),
            (&["\""], "{\"}"),
            (&["a] b"], "{a] b}"),
            (&["{a}"], "{{a}}"),
            (&["a\x0bb"], "{a\x0bb}"),
        ];
        for (items, expected) in cases {
            let vals: Vec<Value> = items.iter().map(|s| Value::from_str(s)).collect();
            assert_eq!(serialize_list(&vals), *expected, "items: {:?}", items);
        }
    }

    #[test]
    fn test_list_round_trip() {
        let items = [
            "a}b", "{a}", "", "\\", "\\n", "a\nb", "\u{0}x", "#hash",
            "a\"b", " lead", "trail ", "x{y}z", "中", "\u{1} ",
        ];
        for item in items {
            let list = Value::from_list(&[Value::from_str(item)]);
            let parsed = list.as_list().unwrap();
            assert_eq!(parsed.len(), 1, "serialized: {}", list.as_str());
            assert_eq!(parsed[0].as_str(), item, "serialized: {}", list.as_str());
        }
    }

    #[test]
    fn test_parse_malformed_lists() {
        assert!(Value::from_str("{").as_list().is_none());
        assert!(Value::from_str("a {b").as_list().is_none());
        assert!(Value::from_str("\"abc").as_list().is_none());
        assert!(Value::from_str("{a}b").as_list().is_none());
        assert!(Value::from_str("\"a\"b").as_list().is_none());
        let err = Value::from_str("{").as_list_strict().unwrap_err();
        assert_eq!(err.message, "unmatched open brace in list");
        assert_eq!(err.code, "TCL VALUE LIST BRACE");
        let err = Value::from_str("\"a").as_list_strict().unwrap_err();
        assert_eq!(err.message, "unmatched open quote in list");
        let err = Value::from_str("{a}b").as_list_strict().unwrap_err();
        assert_eq!(err.message, "list element in braces followed by \"b\" instead of space");
        assert_eq!(err.code, "TCL VALUE LIST JUNK");
    }

    #[test]
    fn test_parse_backslash_collapse() {
        let v = Value::from_str("a\\}b");
        let list = v.as_list().unwrap();
        assert_eq!(list[0].as_str(), "a}b");
        let v = Value::from_str("a\\nb");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a\nb");
        let v = Value::from_str("a\\x41b");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "aAb");
        let v = Value::from_str("q\\q");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "qq");
        // Trailing backslash in a bare element is kept.
        let v = Value::from_str("a\\");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a\\");
        // Backslash-newline folds to a single space.
        let v = Value::from_str("a\\\n  b");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a b");
    }

    #[test]
    fn test_parse_braced_literal_backslashes() {
        // Inside braces, backslashes are literal (only affect brace counting).
        let v = Value::from_str("{a\\}b}");
        let list = v.as_list().unwrap();
        assert_eq!(list[0].as_str(), "a\\}b");
        // `{\}` is an unmatched brace: the \} does not close the element.
        assert!(Value::from_str("{\\}").as_list().is_none());
    }
}
