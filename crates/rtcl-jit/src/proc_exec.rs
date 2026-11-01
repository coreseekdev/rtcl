//! proc-body JIT 钩子层（Lane A P1 瘦身后）：install + 策略开关 +
//! 引擎分派。fast 机器在 crate::fast，classic 池在 crate::classic，
//! 宿主方法在 crate::host/exec。

use crate::abi;
use crate::classic::compile_cached;
use crate::exec;
use rtcl_core::{Interp, Value};
use rtcl_vm::interp::set_proc_body_executor;
use rtcl_ir::ByteCode;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

/// fast-only 策略开关（RTCL_JIT_POOL=1 时恒 false——全开）。
static FAST_ONLY: AtomicBool = AtomicBool::new(false);

/// 池 key 三元组（interp 指针, proc 体 Rc 指针, epoch）。
/// 注册 proc-body 钩子（进程级一次性；`jitsh` main 调用）。
pub fn install() {
    set_proc_body_executor(Some(exec_proc_body));
}
/// 置位 fast-only（进程级，engine 注册时调用一次）。
pub fn set_fast_only(v: bool) {
    FAST_ONLY.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn fast_only() -> bool {
    if crate::env_value("RTCL_JIT_POOL").as_deref() == Some("1") {
        return false;
    }
    FAST_ONLY.load(std::sync::atomic::Ordering::Relaxed)
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
    if crate::env_value("RTCL_JIT_FAST").as_deref() != Some("0") {
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
    if !crate::aggressive_tier()
        && crate::env_value("RTCL_JIT_POOL").as_deref() != Some("1")
    {
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
        let key = (Rc::as_ptr(code) as usize, code.epoch);
        let module = compile_cached(code, key)?;
        return Some(crate::classic::wt::execute_proc_wasmtime(interp, code, &module));
    }
    // wasmi 池优先（同上——命中路径零模块查找）。
    if let Some(r) = crate::classic::wasmi::execute_proc_pooled(interp, code) {
        return Some(r);
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch);
    let module = compile_cached(code, key)?;
    // proc 语境的 case-b 帧协议已落地（rtcl-vm jit_err_* 公共入口 +
    // Host.proc_ctx 路由）——不再按 Cold 退回（f002196/f004860 的
    // (procedure …) 帧链由差分验证）。
    Some(crate::classic::wasmi::execute_proc(interp, code, &module))
}
