//! classic 池子系统（Lane A P1——自 proc_exec.rs 迁出）：
//! emit 模块缓存（PROC_MODULES/compile_cached）+ wasmi/wasmtime 两个
//! classic proc 执行池。fast-tier 经 classic::compile_cached 取模块
//! 做 gate 判定（同一编译，零浪费）。

use crate::emit;
use crate::pass::EmitConfig;
use rtcl_ir::ByteCode;
use std::rc::Rc;

/// 池 key 三元组（interp 指针, proc 体 Rc 指针, epoch）。
pub(crate) type PoolKey = (usize, usize, u64, u64);

thread_local! {
    /// proc 体的发射产物缓存（M4「定义即编译」的雏形）：key =
    /// (ByteCode 的 Rc 指针, epoch)——指针稳定性由 ProcDef.compiled 的
    /// 长期 Rc 保证；redefine 换新 Rc 且 epoch 变化，双重防陈旧命中。
    /// 上限 64 条，超出全清（进程级模块数有限；简单粗暴即可）。
    static PROC_MODULES: std::cell::RefCell<
        std::collections::HashMap<(usize, u64, u64), Rc<emit::Module>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Evict the emitted module for a retired compiled unit.
pub(crate) fn retire_bytecode(ptr: usize) {
    PROC_MODULES.with(|c| c.borrow_mut().retain(|k, _| k.0 != ptr));
}

/// Emitted-module cache keys carry no interpreter identity (they are
/// shared per thread), so a teardown clears the whole cache: entries can
/// keep Rc<emit::Module> and ByteCode-derived metadata alive after the
/// owning interpreter is gone.
pub(crate) fn teardown_interp(_ptr: usize) {
    PROC_MODULES.with(|c| c.borrow_mut().clear());
}

/// emit 编译缓存（key 二元组；未命中现编译，拒绝不缓存负项）。
pub(crate) fn compile_cached(code: &ByteCode, key: (usize, u64, u64)) -> Option<Rc<emit::Module>> {
    PROC_MODULES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(m) = c.get(&key) {
            return Some(Rc::clone(m));
        }
        // PR#2 融合超指令展开（BEGIN_CMD_PUSH_CONST / FOREACH_NEXT_CLOSE
        // / CMP_VI_JF …）——JIT 消费 unfused 规范形状（run_top_with
        // 同款）。
        let bc = code.unfused();
        match emit::compile(&bc, &EmitConfig::default()) {
            Ok(mm) => {
                if c.len() >= 64 {
                    c.clear();
                }
                let m = Rc::new(mm);
                c.insert(key, Rc::clone(&m));
                Some(m)
            }
            Err(_) => None,
        }
    })
}

pub(crate) mod wasmi;
#[cfg(feature = "jit-native")]
pub(crate) mod wt;
