//! proc-body JIT 钩子层（Lane A P1 瘦身后）：install + 策略开关 +
//! 引擎分派。fast 机器在 crate::fast，classic 池在 crate::classic，
//! 宿主方法在 crate::host/exec。

use crate::abi;
use crate::classic::compile_cached;
use crate::exec;
use rtcl_core::{Interp, Value};
use rtcl_vm::interp::{
    set_bytecode_retire_hook, set_interp_teardown_hook, set_proc_body_executor,
    set_proc_precompile_hook,
};
use rtcl_ir::ByteCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

/// fast-only 策略开关（RTCL_JIT_POOL=1 时恒 false——全开）。
static FAST_ONLY: AtomicBool = AtomicBool::new(false);

/// 池 key 三元组（interp 指针, proc 体 Rc 指针, epoch）。
/// 注册 proc-body 钩子（进程级一次性；`jitsh` main 调用）。
pub fn install() {
    set_proc_body_executor(Some(exec_proc_body));
    set_proc_precompile_hook(Some(autocompile_proc));
    set_interp_teardown_hook(Some(teardown_interp));
    set_bytecode_retire_hook(Some(retire_bytecode));
}

/// A proc's compiled unit is being dropped: evict module/pool entries
/// keyed by its address so redefinition does not retain old bodies until
/// the cache cap.
fn retire_bytecode(code: *const rtcl_ir::ByteCode) {
    let ptr = code as usize;
    crate::classic::wasmi::retire_bytecode(ptr);
    #[cfg(feature = "jit-native")]
    crate::classic::wt::retire_bytecode(ptr);
    crate::classic::retire_bytecode(ptr);
    crate::fast::wasmi::retire_bytecode(ptr);
    #[cfg(feature = "jit-native")]
    crate::fast::wt::retire_bytecode(ptr);
}

/// `Interp` drop hook: release thread-local pools/modules keyed by the
/// dying interpreter (classic wasmi/wasmtime pools + fast store/modules +
/// shared emit cache).
fn teardown_interp(interp: *mut Interp) {
    // M4 统计挂点（env 门控 + 一次性守卫；exit 路径由 jitsh 显式补）。
    dump_autocompile_stats();
    let ptr = interp as usize;
    crate::classic::wasmi::teardown_interp(ptr);
    #[cfg(feature = "jit-native")]
    crate::classic::wt::teardown_interp(ptr);
    crate::classic::teardown_interp(ptr);
    crate::fast::wasmi::teardown_interp(ptr);
    #[cfg(feature = "jit-native")]
    crate::fast::wt::teardown_interp(ptr);
}
/// 置位 fast-only（进程级，engine 注册时调用一次）。
pub fn set_fast_only(v: bool) {
    FAST_ONLY.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// RTCL_JIT_POOL=1（classic 池显式 opt-in）。每 proc 调用两次的
/// getenv → 进程级一次读（环境在进程生命周期内不变；judge/battery/
/// bench 均为进程启动前设定——差分回归逃生口语义不受影响）。
fn pool_optin() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| crate::env_value("RTCL_JIT_POOL").as_deref() == Some("1"))
}

/// RTCL_JIT_FAST=0 关闭 fast tier（差分回归逃生口；默认开启）。
/// 同上：每调用一次 getenv → 一次读。
pub(crate) fn fast_tier_enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| crate::env_value("RTCL_JIT_FAST").as_deref() != Some("0"))
}

pub(crate) fn fast_only() -> bool {
    if pool_optin() {
        return false;
    }
    FAST_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// M4「定义即编译」（Chez/Racket CS 模式，docs/JIT-WASM-GC.md §7.4/§9）
// ---------------------------------------------------------------------------

/// 默认档（进程级）：`jitsh` main 在 install 前置 true（jitsh 是测试
/// 驱动，全门禁经 jitsh 走预编译路径）；rtcl-cli（--features jit）与
/// 库用户保持 false——环境变量 `RTCL_JIT_AUTOCOMPILE=1` 灰度 opt-in
/// （bench 证伪风险后再翻默认）。
static AUTO_COMPILE: AtomicBool = AtomicBool::new(false);

/// 置定义点预编译默认档（进程启动期调用一次；env 显式值优先）。
pub fn set_autocompile(v: bool) {
    AUTO_COMPILE.store(v, Ordering::Relaxed);
}

/// 定义点预编译开关。env `RTCL_JIT_AUTOCOMPILE` 显式值优先（"1" 开，
/// 其余关——差分回归逃生口），否则取进程默认档。OnceLock 一次读
/// （env 在进程生命周期内不变语义不变，同 RTCL_JIT_POOL 口径）。
fn autocompile() -> bool {
    static ENV: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    match *ENV.get_or_init(|| crate::env_value("RTCL_JIT_AUTOCOMPILE").map(|v| v == "1")) {
        Some(v) => v,
        None => AUTO_COMPILE.load(Ordering::Relaxed),
    }
}

/// 定义点预编译的 op 预算护栏（§8 假设 C：<1ms/proc 的同步编译上界；
/// §7.2 核算的 50-200 op 常规体在微秒级，此阈值只兜超大模块——Chez/
/// Racket CS 的「解释器兜底超大输入」对应物，PLT_CS_COMPILE_LIMIT 的
/// rtcl 版）。超限体跳过预编译，保持既有调用时编译路径（语义零变化
/// ——只是把编译延迟留在首调用）。融合展开（unfused ≈ ≤2×）后仍在
/// 万级 op、发射 μs 量级，余量充足。
const PRECOMPILE_OPS_LIMIT: usize = 4096;

/// 覆盖面统计（env `RTCL_JIT_AUTOCOMPILE_STATS` 时 dump 到 stderr）。
static AC_ACCEPTED: AtomicUsize = AtomicUsize::new(0);
static AC_REJECTED: AtomicUsize = AtomicUsize::new(0);
static AC_SKIPPED: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    /// (emit 纳秒, 融合 op 数) 样本，上限 4096（够分布统计；溢出停采）。
    static AC_SAMPLES: std::cell::RefCell<Vec<(u32, u32)>>
        = std::cell::RefCell::new(Vec::with_capacity(512));
    /// 拒绝原因普查（NYI 清单）：`Unsupported` 文本 → 计数。文本含
    /// pc 等高基数量词——精确去重 + 64 桶上限，溢出归 `…(other)`。
    static AC_REASONS: std::cell::RefCell<Vec<(String, usize)>>
        = std::cell::RefCell::new(Vec::new());
}
static AC_DUMPED: AtomicBool = AtomicBool::new(false);

/// 记一条拒绝原因（精确文本去重，64 桶上限，溢出并桶）。
fn record_reject_reason(reason: &str) {
    AC_REASONS.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(e) = r.iter_mut().find(|(s, _)| s == reason) {
            e.1 += 1;
            return;
        }
        if r.len() >= 64 {
            if let Some(e) = r.iter_mut().find(|(s, _)| s == "…(other)") {
                e.1 += 1;
            } else {
                r.push(("…(other)".to_string(), 1));
            }
            return;
        }
        r.push((reason.to_string(), 1));
    });
}

/// 钩子体（`cmd_proc` 定义点）：门已由运行侧（fallback/epoch/
/// bytecode_applicable）把过，此处只管发射 + 缓存 + 计量。
/// - 消费面检查：fast tier / classic 池 / aggressive 全关时模块无人
///   消费（exec_proc_body 在 compile_cached 之前就回落解释器）——
///   预发射纯浪费，跳过。
/// - 预算护栏：超大体跳过（见 [`PRECOMPILE_OPS_LIMIT`]）。
/// - 发射进 PROC_MODULES（compile_cached 同一入口：命中零编译；拒绝
///   落负缓存——调用面不再重试）。
fn autocompile_proc(_interp: &mut Interp, code: &Rc<ByteCode>) {
    if !autocompile() {
        return;
    }
    // 消费面检查（与 exec_proc_body 的回落序一致）。
    if !fast_tier_enabled() && !pool_optin() && !crate::aggressive_tier() {
        return;
    }
    let ops = code.ops().len();
    if ops > PRECOMPILE_OPS_LIMIT {
        AC_SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch, code.serial);
    let t0 = Instant::now();
    let ok = compile_cached(code, key).is_some();
    let ns = t0.elapsed().as_nanos() as u32;
    if ok {
        AC_ACCEPTED.fetch_add(1, Ordering::Relaxed);
    } else {
        AC_REJECTED.fetch_add(1, Ordering::Relaxed);
        // 拒绝原因普查：负缓存吞掉了 Unsupported 文本，重新发射一次取
        // 原因（拒绝是每 proc 一次的路径，μs 级重复发射可忽略）。
        let bc = code.unfused();
        if let Err(e) = crate::emit::compile(&bc, &crate::pass::EmitConfig::default()) {
            record_reject_reason(&e.to_string());
        }
    }
    AC_SAMPLES.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < 4096 {
            s.push((ns, ops as u32));
        }
    });
}

/// 定义点编译统计 dump（env `RTCL_JIT_AUTOCOMPILE_STATS` 才输出）：
/// 覆盖面（accepted/rejected/budget-skipped）+ emit 耗时分布
/// （p50/p90/max）+ 耗时-op 关系极值。进程级一次性（AC_DUMPED 守卫
/// ——jitsh 的 exit 路径与 Interp teardown 双挂点只出一份）。
pub fn dump_autocompile_stats() {
    if crate::env_value("RTCL_JIT_AUTOCOMPILE_STATS") != Some("1".to_string()) {
        return;
    }
    if AC_DUMPED.swap(true, Ordering::Relaxed) {
        return;
    }
    let accepted = AC_ACCEPTED.load(Ordering::Relaxed);
    let rejected = AC_REJECTED.load(Ordering::Relaxed);
    let skipped = AC_SKIPPED.load(Ordering::Relaxed);
    let mut samples = AC_SAMPLES.with(|s| std::mem::take(&mut *s.borrow_mut()));
    samples.sort_unstable();
    let pct = |p: f64| -> u32 {
        if samples.is_empty() {
            return 0;
        }
        let idx = (((samples.len() as f64) * p).floor() as usize).min(samples.len() - 1);
        samples[idx].0
    };
    let max_ops = samples.last().map(|(_, o)| *o).unwrap_or(0);
    eprintln!(
        "[autocompile] accepted={} rejected={} budget_skipped={} total={} \
         emit_ns p50={} p90={} max={} max_ops={}",
        accepted,
        rejected,
        skipped,
        accepted + rejected + skipped,
        pct(0.50),
        pct(0.90),
        pct(1.0),
        max_ops,
    );
    AC_REASONS.with(|r| {
        let r = r.borrow();
        if r.is_empty() {
            return;
        }
        let mut rows: Vec<(String, usize)> = r.clone();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (s, n) in &rows {
            eprintln!("[autocompile-reject] {} x{}", s, n);
        }
    });
}
/// 钩子体：接受则 JIT 执行，`None` 退回 vm_exec。
fn exec_proc_body(interp: &mut Interp, code: &Rc<ByteCode>) -> Option<rtcl_core::Result<Value>> {
    // epoch 门（M3 口径——Site A 的 guard 已查过，此处防御性重查：
    // 钩子与门之间的窗口内可能发生 Tier1 bump）。
    if code.epoch != interp.tier1_epoch() {
        return None;
    }
    // fast-tier 优先（MoonBit 式直呼；RTCL_JIT_FAST=0 关闭）。引擎分派：
    // wasmi → FAST_STORE 驱动；wasmtime → WT_FAST_STORE 驱动（exec_fast_wt，
    // 与 wasmi 同构）。槽命中零编译查找；未命中 compile_cached（常规冷
    // 路径也需要同一编译——零浪费）。
    // 默认开启（RTCL_JIT_FAST=0 关闭——差分回归逃生口）。judge 88/88
    // （fast=1）+ battery/probes + fib(2)-(30) 全深度三方一致后转正。
    if fast_tier_enabled() {
        #[cfg(feature = "jit-native")]
        if crate::exec::engine_from_env() == crate::exec::EngineKind::Wasmtime {
            if let Some(r) = crate::fast::wt::fast_try_wt(interp, code) {
                return Some(r);
            }
        } else if let Some(r) = crate::fast::wasmi::fast_try(interp, code) {
            return Some(r);
        }
        #[cfg(not(feature = "jit-native"))]
        if let Some(r) = crate::fast::wasmi::fast_try(interp, code) {
            return Some(r);
        }
    }
    // fast-only：非 fast 形状到此为止——退回解释器（None →
    // exec_bytecode），常规池路径仅在显式 RTCL_JIT_POOL=1 时走。
    if fast_only() {
        return None;
    }
    // 非 fast 体默认回落 exec_bytecode（rtcl 原生解释器）——classic
    // wasm 模块的逐命令宿主分派对 dispatch 密集体 3-7× 慢于原生
    // （tax1 foreach 6.9×/tax2 for+lindex 3.1× 实测；此前注释声称
    // 「常规池仅在 RTCL_JIT_POOL=1 时走」但代码从未兑现）。classic
    // 池 = 显式 opt-in（RTCL_JIT_POOL=1，旧行为全量保留）。
    // aggressive（wasm32）：JIT 产物宿主引擎原生编译 vs 解释器 wasm
    // 化——classic 保留（接近即保留，用户指令）。
    if !crate::aggressive_tier() && !pool_optin() {
        return None;
    }
    // 引擎分派（RTCL_JIT_ENGINE；默认 wasmi）。wasmtime 走 Cranelift
    // 池化路径（Engine 单例 + Module 缓存 + Store/Instance 池）。
    #[cfg(feature = "jit-native")]
    if crate::exec::engine_from_env() == crate::exec::EngineKind::Wasmtime {
        // 池优先：命中 = 该单元此前已被接受（池条目只在编译成功后
        // 存在）——免 PROC_MODULES/WASMI_MODULES 两次哈希查找
        //（fib 差分剖析：每调用 ~80ns 的纯开销）。
        if let Some(r) = crate::classic::wt::execute_proc_wasmtime_pooled(interp, code) {
            return Some(r);
        }
        let key = (Rc::as_ptr(code) as usize, code.epoch, code.serial);
        let module = compile_cached(code, key)?;
        return Some(crate::classic::wt::execute_proc_wasmtime(interp, code, &module));
    }
    // wasmi 池优先（同上——命中路径零模块查找）。
    if let Some(r) = crate::classic::wasmi::execute_proc_pooled(interp, code) {
        return Some(r);
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch, code.serial);
    let module = compile_cached(code, key)?;
    // proc 语境的 case-b 帧协议已落地（rtcl-vm jit_err_* 公共入口 +
    // Host.proc_ctx 路由）——不再按 Cold 退回（f002196/f004860 的
    // (procedure …) 帧链由差分验证）。
    Some(crate::classic::wasmi::execute_proc(interp, code, &module))
}
