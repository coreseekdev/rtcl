//! 宿主字符串池 trait 层（lazy-string，docs/JIT-WASM-GC.md §6）。
//!
//! **机制**：JS（或其他宿主）起源的字符串默认保持宿主原生形态——值里只
//! 持有一个池索引（[`crate::value::InternalRep::HostStr`]），首次需要字节
//! 级语义时才经 [`HostStringBacking::to_bytes`] 物化为 rtcl 原生 UTF-8；
//! 反向（Tcl→宿主）同样惰性——原生值首次穿越边界时经
//! [`HostStringBacking::insert`] 建宿主形态并缓存池索引（值进入 Dual
//! 态：宿主串 + 已物化字节并存，二次穿越零成本）。
//!
//! **接线**（对齐 `host.rs` 的 trait 层模式）：rtcl-vm 不认识 JS。wasm32
//! 由 rtcl-wasm 注册 `JsStringBacking`（`Vec<Option<JsString>>` + 空闲栈，
//! js_sys externref 表保活）；非 JS 宿主无人注册 → [`InternalRep::
//! HostStr`](crate::value::InternalRep) 编译期无人构造、运行期不出现——
//! 注册表即开关（§8 开关矩阵的 `lazy-string`，rtcl-wasm 默认 on 的实现
//! 形态），不需要 per-interp option：宿主形态是进程级事实，不是解释器
//! 配置。
//!
//! **状态机**（值的三态）：
//!
//! ```text
//! JS→Tcl：JsOnly（HostStr(idx)，字节 cell 空）
//!          ──首次字节操作（as_str choke point）──→
//!         Dual（HostStr(idx) + 字节 cell 已物化）
//!          ──rep 变异（set_int_rep 等）──→ NativeOnly（池引用已释放）
//! Tcl→JS：NativeOnly ──Value::to_host_str 首次穿越──→ Dual（缓存池索引）
//! ```
//!
//! **池引用计数**：`HostStr(idx)` 的所有权规则与堆箱一致——构造持 1 引用；
//! `InternalRep::clone`（COW 深拷贝路径）经 [`HostStringBacking::acquire`]
//! 加 1；`Drop` 或 rep 覆写经 [`HostStringBacking::release`] 减 1，归零的
//! 槽位回收。宿主串生命周期因此严格跟随值的生命周期，跨边界即移交所有
//! 权（§10 风险 3 的池侧对应面）。
//!
//! **白名单与强制物化**（§6 逐条验证结论，落地见各调用点）：
//!
//! | 操作 | 落点 | 结论 |
//! |---|---|---|
//! | `string equal` / `eq` / `ne` | `string_cmds.rs` / `expr_ops::op_str_eq` | **生效**。UTF-8↔UTF-16 对合法标量值双射，深层相等免物化 |
//! | concat / `string cat` | `string_cmds.rs` | **生效**。「拼接→转码 ≡ 转码→拼接」对合法标量值成立 |
//! | 回传 JS / puts 到 console | rtcl-wasm 边界 / `HostConsole::write_value_stdout` | **生效**。零转码，收益本体 |
//! | ~~`string compare`~~ | —— | **强制物化，不进白名单**。§6 原论证有误：UTF-16 码元序**不**保码点序（astral 字符的高位代理 D800–DBFF < U+E000..U+FFFF，码点序却相反）；rtcl 现状 = UTF-8 字节序（= 码点序），若走 JS `<` 比较会使 HostStr 值与物化值在 astral↔U+E000..U+FFFF 混合内容上得出不同次序——静默语义分歧，与「ground truth = 解释器」矛盾。等 specs astral probe + judge 裁决后方再评估 |
//! | `string length` | —— | **强制物化**（§6 既定）：JS `.length` 数 UTF-16 码元、tclsh 8.6 UCS-2、rtcl 现状疑似 code point，三方未裁决前不动 |
//!
//! 其余一切（list/dict parse、`string map/match/range/index/…`、regexp、
//! expr 数值解析、binary、eval/替换）都不需要显式代码：它们全部经
//! `Value::as_str` 唯一入口取字节，choke point 物化即强制物化清单的执行。
//!
//! **lone surrogate 防线**（§6）：池中永不出现含孤立代理的宿主串——入池
//! （宿主的 insert 路径）由 backing 实现负责一次校验，含孤立代理的串立即
//! 转码 + 替换符（U+FFFD）兜底、以原生字节形态进值。白名单的安全性论证
//! 全部建立在「合法标量值」前提上；rtcl-vm 侧不再重复校验（契约见下）。

use smallvec::SmallVec;
use std::cell::RefCell;
use std::rc::Rc;

/// 物化后的字节形态（与 `value.rs` 的 string cell 同型：内联 23 字节）。
pub type HostStrBytes = SmallVec<[u8; 23]>;

/// 宿主字符串池的背板 trait（rtcl-vm 不认识 JS；wasm32 由 rtcl-wasm 注册
/// `JsStringBacking`）。
///
/// # 契约
///
/// - `idx` 由 backing 签发，pool 槽位带引用计数：`insert`/`concat` 返回的
///   索引以 1 引用起算，所有权移交调用方（进 `InternalRep::HostStr`）。
/// - `acquire`/`release` 必须配平；归零槽位立即回收复用（空闲栈）。
/// - **入池过滤**：`insert` 的调用方保证字节是合法 UTF-8；宿主原生串入池
///   路径（wasm 侧 JS→Tcl 边界）必须先做孤立代理校验——含孤立代理的串
///   不得入池（转码 + U+FFFD 兜底后走原生字节形态）。池内容因此恒为合法
///   标量值序列，白名单的安全性论证依赖此不变量。
/// - 实现不得重入 [`with_backing`]（会撞 RefCell 借用）。
pub trait HostStringBacking {
    /// JsOnly → Dual 的物化：宿主原生形态 → rtcl UTF-8 字节（UTF-16→UTF-8
    /// 转码，白名单外的首次字节语义才走这里）。
    fn to_bytes(&self, idx: u32) -> HostStrBytes;
    /// 免物化深层相等（`string equal`/`eq`/`ne` 白名单）。对合法标量值，
    /// UTF-8 字节相等 ⇔ UTF-16 码元序列相等。
    fn equal(&self, a: u32, b: u32) -> bool;
    /// 免物化拼接 → 新池索引（concat / `string cat` 白名单）。拼接→转码 ≡
    /// 转码→拼接（合法标量值）。**借用语义**：不动 `a`/`b` 的引用计数（输
    /// 入方仍各自持有）；返回新槽（1 引用起算，所有权移交调用方）——折叠
    /// 拼接的中间槽由调用方显式 `release`。
    fn concat(&self, a: u32, b: u32) -> u32;
    /// 反向入池（Tcl→JS 双缓存）：UTF-8 字节 → 新池索引（1 引用起算）。
    fn insert(&self, bytes: &[u8]) -> u32;
    /// 免物化字节空判（`Value::is_empty` 对 JsOnly 态的支撑；合法串的字节
    /// 空 ⇔ 宿主形态空）。
    fn is_empty(&self, idx: u32) -> bool;
    /// COW：`InternalRep::HostStr` 克隆（`ValueInner` 深拷贝路径）加引用。
    fn acquire(&self, idx: u32);
    /// `InternalRep::HostStr` 释放（值回收或 rep 覆写）减引用；归零回收。
    fn release(&self, idx: u32);
}

thread_local! {
    /// 进程级（每线程）注册表。rtcl-wasm 在构造首个 handle 前注册一次；
    /// 其余平台永不注册。`Rc` 非 Send——值本身 !Send，宿主串形态（js_sys
    /// `JsString`）同样线程绑定，正好同域。
    static BACKING: RefCell<Option<Rc<dyn HostStringBacking>>> =
        const { RefCell::new(None) };
}

/// 注册背板（rtcl-wasm 的 `JsStringBacking` 在构造解释器前调用）。
/// 幂等覆盖：再次注册替换旧背板——此时若有存活 HostStr 值引用旧池，旧池
/// 由旧 `Rc` 保活，值仍可物化/释放（泄漏到进程退出为止）；契约上宿主应
/// 只注册一次。
pub fn register_host_string_backing(backing: Rc<dyn HostStringBacking>) {
    // 注册失败（线程 teardown 中）= 无值存活的死线程，注册本身无意义。
    let _ = BACKING.try_with(|slot| *slot.borrow_mut() = Some(backing));
}

/// 是否已注册背板（开关探测：rtcl-wasm 的收益断言用）。
pub fn host_string_backing_registered() -> bool {
    BACKING.try_with(|slot| slot.borrow().is_some()).unwrap_or(false)
}

/// 以已注册背板运行 `f`；未注册（或线程 teardown 中）返回 `None`。
///
/// 这是惰性钩子的唯一取背板入口：调用点全部在 HostStr 值的冷路径上
/// （物化/白名单/池引用增减），原生值的热路径（string cell 已填充的
/// `as_str`、非 HostStr 的 rep 分派）不经过这里。
#[inline]
pub fn with_backing<R>(f: impl FnOnce(&dyn HostStringBacking) -> R) -> Option<R> {
    BACKING.try_with(|slot| {
        let b = slot.borrow();
        b.as_deref().map(f)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
pub(crate) mod test_support {
    //! 测试期开关：`scoped_backing` 守卫注册 fake 背板并在 drop 时摘除，
    //! 隔离各测试线程的注册表（cargo test 每测试独立线程 → 独立 TLS）。
    use std::rc::Rc;

    /// RAII：构造即注册，drop 即摘除（恢复 None）。
    pub struct ScopedBacking {
        _backing: Rc<dyn super::HostStringBacking>,
    }

    impl ScopedBacking {
        pub fn new(backing: Rc<dyn super::HostStringBacking>) -> Self {
            super::register_host_string_backing(backing.clone());
            ScopedBacking { _backing: backing }
        }
    }

    impl Drop for ScopedBacking {
        fn drop(&mut self) {
            // 摘除 = 置回 None（不是替换：同线程后续测试不得看见 fake）。
            let _ = super::BACKING.try_with(|slot| *slot.borrow_mut() = None);
        }
    }
}

// ── fake 背板驱动的全状态机测试 ─────────────────────────────────
//
// 用一个可观测的 fake 池（槽位 + 引用计数 + 空闲栈 + 物化/入池/拼接计
// 数器）驱动 §6 状态机的每一条转移边；eval 级测试同时验证白名单/强制
// 物化清单在命令面上的落点。

#[cfg(test)]
mod tests {
    use super::test_support::ScopedBacking;
    use super::*;
    use crate::value::Value;
    use std::cell::Cell;

    /// 可观测 fake 池：语义 = 简化版 JsStringBacking（String 代替
    /// JsString；release 归零回收槽位进空闲栈）。
    struct FakePool {
        slots: std::cell::RefCell<Vec<Option<String>>>,
        rc: std::cell::RefCell<Vec<u32>>,
        free: std::cell::RefCell<Vec<u32>>,
        transcodes: Cell<usize>,
        inserts: Cell<usize>,
        concats: Cell<usize>,
    }

    impl FakePool {
        fn new() -> Rc<Self> {
            Rc::new(FakePool {
                slots: std::cell::RefCell::new(Vec::new()),
                rc: std::cell::RefCell::new(Vec::new()),
                free: std::cell::RefCell::new(Vec::new()),
                transcodes: Cell::new(0),
                inserts: Cell::new(0),
                concats: Cell::new(0),
            })
        }
        fn insert_raw(&self, s: String) -> u32 {
            self.inserts.set(self.inserts.get() + 1);
            if let Some(idx) = self.free.borrow_mut().pop() {
                let i = idx as usize;
                self.slots.borrow_mut()[i] = Some(s);
                self.rc.borrow_mut()[i] = 1;
                idx
            } else {
                let idx = self.slots.borrow().len() as u32;
                self.slots.borrow_mut().push(Some(s));
                self.rc.borrow_mut().push(1);
                idx
            }
        }
        fn rc_of(&self, idx: u32) -> u32 {
            self.rc.borrow()[idx as usize]
        }
        fn live(&self, idx: u32) -> bool {
            self.slots.borrow()[idx as usize].is_some()
        }
    }

    impl HostStringBacking for FakePool {
        fn to_bytes(&self, idx: u32) -> HostStrBytes {
            self.transcodes.set(self.transcodes.get() + 1);
            HostStrBytes::from_slice(
                self.slots.borrow()[idx as usize].as_ref().unwrap().as_bytes(),
            )
        }
        fn equal(&self, a: u32, b: u32) -> bool {
            self.slots.borrow()[a as usize] == self.slots.borrow()[b as usize]
        }
        fn concat(&self, a: u32, b: u32) -> u32 {
            self.concats.set(self.concats.get() + 1);
            let s = format!(
                "{}{}",
                self.slots.borrow()[a as usize].as_ref().unwrap(),
                self.slots.borrow()[b as usize].as_ref().unwrap()
            );
            self.insert_raw(s)
        }
        fn insert(&self, bytes: &[u8]) -> u32 {
            self.insert_raw(String::from_utf8_lossy(bytes).into_owned())
        }
        fn is_empty(&self, idx: u32) -> bool {
            self.slots.borrow()[idx as usize].as_ref().unwrap().is_empty()
        }
        fn acquire(&self, idx: u32) {
            self.rc.borrow_mut()[idx as usize] += 1;
        }
        fn release(&self, idx: u32) {
            let i = idx as usize;
            let mut rc = self.rc.borrow_mut();
            rc[i] -= 1;
            if rc[i] == 0 {
                self.slots.borrow_mut()[i] = None;
                self.free.borrow_mut().push(idx);
            }
        }
    }

    /// 注册 fake 并返回句柄（RAII 摘除，隔离线程 TLS）。
    fn with_fake() -> (ScopedBacking, Rc<FakePool>) {
        let pool = FakePool::new();
        let guard = ScopedBacking::new(pool.clone());
        (guard, pool)
    }

    #[test]
    fn no_backing_registered_by_default() {
        // 每个测试线程的注册表初始为空（非 JS 宿主 = 运行期零构造）。
        assert!(!host_string_backing_registered());
        // 原生值/立即数在无背板下没有宿主句柄。
        assert_eq!(Value::from_str("abc").host_str_handle(), None);
        assert_eq!(Value::from_int(7).host_str_handle(), None);
        // 未注册时 to_host_str 也是 None（Tcl→JS 穿越关闭）。
        assert_eq!(Value::from_str("abc").to_host_str(), None);
    }

    #[test]
    fn js_only_materializes_once_choke_point() {
        let (_g, pool) = with_fake();
        let idx = pool.insert_raw("hello".to_string());
        let v = Value::from_host_str(idx);
        // JsOnly：句柄可见，字节未物化。
        assert_eq!(v.host_str_handle(), Some(idx));
        assert_eq!(pool.transcodes.get(), 0);
        assert_eq!(v.as_str(), "hello");
        assert_eq!(pool.transcodes.get(), 1); // JsOnly → Dual，恰好一次
        assert_eq!(v.as_str(), "hello");
        assert_eq!(pool.transcodes.get(), 1); // Dual：字节缓存，零重复物化
        // 白名单操作仍可用（Dual 态 equal 与字节缓存无关）。
        assert_eq!(v.as_str().len(), 5);
    }

    #[test]
    fn is_empty_js_only_without_materialization() {
        let (_g, pool) = with_fake();
        let empty_idx = pool.insert_raw(String::new());
        let v = Value::from_host_str(empty_idx);
        assert!(v.is_empty());
        assert_eq!(pool.transcodes.get(), 0); // 免物化问池
        let idx = pool.insert_raw("x".to_string());
        assert!(!Value::from_host_str(idx).is_empty());
    }

    #[test]
    fn whitelist_eq_and_expr_eq_no_materialization() {
        let (_g, pool) = with_fake();
        let a = pool.insert_raw("abc".to_string());
        let b = pool.insert_raw("abc".to_string());
        let d = pool.insert_raw("xyz".to_string());
        assert_ne!(a, b); // 独立槽位（内容相等≠同槽）
        let va = Value::from_host_str(a);
        let vb = Value::from_host_str(b);
        let vd = Value::from_host_str(d);
        assert!(crate::types::expr_ops::op_str_eq(&va, &vb).as_int() == Some(1));
        assert!(crate::types::expr_ops::op_str_eq(&va, &vd).as_int() == Some(0));
        assert!(crate::types::expr_ops::op_str_ne(&va, &vd).as_int() == Some(1));
        assert_eq!(pool.transcodes.get(), 0); // 白名单全程零物化

        // 命令面：string equal 裸形同走免物化路径。
        let mut interp = crate::interp::Interp::new();
        interp.set_var("h1", va.clone()).unwrap();
        interp.set_var("h2", vb.clone()).unwrap();
        interp.set_var("h3", vd.clone()).unwrap();
        assert_eq!(interp.eval("string equal $h1 $h2").unwrap().as_int(), Some(1));
        assert_eq!(interp.eval("string equal $h1 $h3").unwrap().as_int(), Some(0));
        assert_eq!(interp.eval("expr {$h1 eq $h3}").unwrap().as_int(), Some(0));
        assert_eq!(pool.transcodes.get(), 0);
    }

    #[test]
    fn mixed_eq_falls_back_to_materialization() {
        let (_g, pool) = with_fake();
        let a = pool.insert_raw("abc".to_string());
        let va = Value::from_host_str(a);
        let native = Value::from_str("abc");
        assert!(crate::types::expr_ops::op_str_eq(&va, &native).as_int() == Some(1));
        // 单侧物化（choke point 兜底——语义与免物化一致）。
        assert_eq!(pool.transcodes.get(), 1);
    }

    #[test]
    fn string_cat_folds_on_host_side() {
        let (_g, pool) = with_fake();
        let a = pool.insert_raw("ab".to_string());
        let b = pool.insert_raw("cd".to_string());
        let c = pool.insert_raw("ef".to_string());
        let mut interp = crate::interp::Interp::new();
        interp.set_var("a", Value::from_host_str(a)).unwrap();
        interp.set_var("b", Value::from_host_str(b)).unwrap();
        interp.set_var("c", Value::from_host_str(c)).unwrap();
        let out = interp.eval("string cat $a $b $c").unwrap();
        // 计数断言放在内容验证之前——验证用的 as_str 本身会物化结果。
        assert_eq!(pool.transcodes.get(), 0); // 全程免物化
        assert_eq!(pool.concats.get(), 2);
        // 中间槽已释放（折叠：a⊕b 产出 m1，m1⊕c 产出结果并释放 m1）。
        // 三个原始槽 + m1 + 结果 = 5 次 insert_raw。
        assert_eq!(pool.inserts.get(), 5);
        assert_eq!(out.host_str_handle().is_some(), true); // 结果仍是宿主形态
        assert_eq!(out.as_str(), "abcdef");
    }

    #[test]
    fn string_cat_single_host_arg_shares_value() {
        let (_g, pool) = with_fake();
        let a = pool.insert_raw("ab".to_string());
        let mut interp = crate::interp::Interp::new();
        interp.set_var("a", Value::from_host_str(a)).unwrap();
        let out = interp.eval("string cat $a").unwrap();
        assert_eq!(pool.transcodes.get(), 0);
        assert_eq!(pool.concats.get(), 0);
        assert_eq!(pool.rc_of(a), 1); // 共享克隆 = 同一堆箱（Value 引用计数），池引用不变
        assert_eq!(out.as_str(), "ab"); // 内容验证（物化此刻才发生）
    }

    #[test]
    fn forced_materialization_via_string_length() {
        let (_g, pool) = with_fake();
        let idx = pool.insert_raw("héllo".to_string()); // 6 字节 UTF-8
        let mut interp = crate::interp::Interp::new();
        interp.set_var("h", Value::from_host_str(idx)).unwrap();
        // string length 刻意留在强制物化侧（三方语义分歧未裁决，§6）。
        assert_eq!(interp.eval("string length $h").unwrap().as_int(), Some(5));
        assert_eq!(pool.transcodes.get(), 1);
        // 二次访问吃 Dual 字节缓存。
        let _ = interp.eval("string length $h");
        assert_eq!(pool.transcodes.get(), 1);
    }

    #[test]
    fn list_parse_materializes() {
        let (_g, pool) = with_fake();
        let idx = pool.insert_raw("a b c".to_string());
        let mut interp = crate::interp::Interp::new();
        interp.set_var("h", Value::from_host_str(idx)).unwrap();
        // 强制物化清单：list parse（lindex）必须走字节。
        assert_eq!(interp.eval("lindex $h 1").unwrap().as_str(), "b");
        assert_eq!(pool.transcodes.get(), 1);
    }

    #[test]
    fn to_host_str_reverse_crossing_creates_dual_and_reuses() {
        let (_g, pool) = with_fake();
        let mut v = Value::from_str("native");
        // 首次穿越：insert 入池 + rep 指向槽位（Dual：字节保留）。
        let idx = v.to_host_str().expect("backing registered");
        assert_eq!(pool.inserts.get(), 1);
        assert_eq!(v.host_str_handle(), Some(idx));
        assert_eq!(v.as_str(), "native"); // Dual 字节缓存，无二次物化
        // 二次穿越零成本：同句柄、零新 insert。
        assert_eq!(v.to_host_str(), Some(idx));
        assert_eq!(pool.inserts.get(), 1);
        // imm 值穿越：as_str 渲染后入池。
        let mut n = Value::from_int(42);
        let nidx = n.to_host_str().unwrap();
        assert_eq!(n.as_str(), "42");
        assert_eq!(pool.slots.borrow()[nidx as usize].as_deref(), Some("42"));
    }

    #[test]
    fn drop_releases_pool_ref_and_slot_is_reused() {
        let (_g, pool) = with_fake();
        let idx;
        {
            let v = Value::from_host_str(pool.insert_raw("tmp".to_string()));
            idx = v.host_str_handle().unwrap();
            assert_eq!(pool.rc_of(idx), 1);
            drop(v);
        }
        assert_eq!(pool.rc_of(idx), 0);
        assert!(!pool.live(idx)); // 归零回收
        // 空闲栈复用同槽。
        let idx2 = pool.insert_raw("next".to_string());
        assert_eq!(idx2, idx);
    }

    #[test]
    fn cow_clone_acquires_and_each_side_releases_once() {
        let (_g, pool) = with_fake();
        let mut shared = Value::from_host_str(pool.insert_raw("s".to_string()));
        let idx = shared.host_str_handle().unwrap();
        let clone = shared.clone(); // Value 引用计数——不触碰池
        assert_eq!(pool.rc_of(idx), 1);
        // COW 变异（共享 → 深拷贝 acquire → rep 覆写 release 克隆侧）。
        shared.set_int_rep(5);
        assert_eq!(shared.as_str(), "5");
        assert_eq!(pool.rc_of(idx), 1); // 克隆侧已释放，原值侧仍持有
        assert_eq!(clone.as_str(), "s"); // 克隆保留宿主形态
        assert_eq!(pool.rc_of(idx), 1);
        drop(clone);
        assert_eq!(pool.rc_of(idx), 0);
        assert!(!pool.live(idx));
    }

    #[test]
    fn mutation_to_native_only_releases() {
        let (_g, pool) = with_fake();
        let mut v = Value::from_host_str(pool.insert_raw("7".to_string()));
        let idx = v.host_str_handle().unwrap();
        v.set_int_rep(8); // ──mutation──→ NativeOnly
        assert_eq!(v.host_str_handle(), None);
        assert_eq!(v.as_str(), "8");
        assert_eq!(pool.rc_of(idx), 0);
        assert!(!pool.live(idx));
    }

    #[test]
    fn string_compare_stays_materialized() {
        // 白名单裁决记录：compare 不进白名单（UTF-16 码元序 ≠ rtcl 字节序，
        // 见 host_string.rs 顶部表）——它必须物化。
        let (_g, pool) = with_fake();
        let a = pool.insert_raw("abc".to_string());
        let b = pool.insert_raw("abd".to_string());
        let mut interp = crate::interp::Interp::new();
        interp.set_var("a", Value::from_host_str(a)).unwrap();
        interp.set_var("b", Value::from_host_str(b)).unwrap();
        assert_eq!(interp.eval("string compare $a $b").unwrap().as_int(), Some(-1));
        assert!(pool.transcodes.get() >= 1); // 物化发生了
    }

    #[test]
    fn unregister_leaves_native_paths_untouched() {
        // ScopedBacking 摘除后，新线程状态为 None——原生构建路径与注册前
        // 逐字节一致（无背板 → HostStr 不可能出现）。
        let g_pool = with_fake();
        drop(g_pool);
        assert!(!host_string_backing_registered());
    }
}
