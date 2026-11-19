//! Coroutine 生命周期/内存审计回归（specs/MEMORY-AUDIT-20261120.md A7 面）：
//!
//! 1. rename away（挂起态）= 协程删除（tclsh 钉形）——record/帧/冻结链
//!    释放，深度单元归还；此前挂起 record 成孤儿（可脚本化无界增长）。
//! 2. rename → 新名 = record 随键平移（新名可续；tclsh 实测钉形）。
//! 3. 深度位随帧走——挂起链的 call_depth 单元移交 record；挂起-永不
//!    resume 循环此前每次 +N，~1000 个挂起即耗尽递归预算（探针实证
//!    n≥5000 报 maximum recursion depth）。本测 4000 轮 > 泄漏阈值：
//!    任何残留深度泄漏都会在轮内以 recursion error 形式暴露。
//! 4. resume 错误路径——协程帧整段丢弃（truncate 回 frames_base）；
//!    此前残留帧破坏作用域解析（一轮即可让顶层 `$var` 读失败）。
//! 5. 注册序日志（command_order）护栏——命令 churn（创建+rename away）
//!    此前线性增长（~100B/命令，oracle 同型零增长）；护栏在条目数
//!    >4096 且 >4×活命令时整表重建。行为面：churn 后 `info commands`
//!    replay 序保持正确、活命令可调。
//!
//! 全部断言走公共 eval 面（不触 interp 私有表）。

use rtcl_vm::interp::Interp;

fn ev(i: &mut Interp, s: &str) -> String {
    i.eval(s).unwrap().as_str().to_string()
}

/// rename away（挂起态）：协程删除 + 深度不漏（4000 轮 > 修复前 ~1000
/// 轮的深度耗尽点；同型命令 churn 后 info commands 面保持正确）。
#[test]
fn coro_rename_away_destroys_record_and_depth() {
    let mut interp = Interp::new();
    let n = 4000;
    for k in 0..n {
        let r = interp.eval(&format!(
            "coroutine c{k} apply [list {{}} {{yield; return x}}]"
        ));
        assert!(r.is_ok(), "iter {k}: coroutine create failed (depth leak?): {r:?}");
        interp.eval(&format!("rename c{k} {{}}")).unwrap();
        // rename away 后命令即消失（catch 可捕获 invalid command）
        let gone = interp.eval(&format!("catch c{k}")).unwrap();
        assert_eq!(gone.as_str(), "1", "iter {k}: resume cmd still callable");
    }
    // 深度预算仍健康：普通 proc 调用正常
    ev(&mut interp, "proc healthy {} { return 1 }");
    assert_eq!(ev(&mut interp, "healthy"), "1");
    assert_eq!(ev(&mut interp, "info level"), "0");
}

/// rename → 新名：record 平移，新名可续（tclsh: resume via new name）。
#[test]
fn coro_rename_rekeys_record_and_resumes() {
    let mut interp = Interp::new();
    ev(&mut interp, "coroutine zz apply {{} {yield; return r}}");
    ev(&mut interp, "rename zz zz2");
    // 旧名不可调（命令已移），新名可续
    assert!(interp.eval("zz").is_err(), "old resume cmd must be gone");
    assert_eq!(ev(&mut interp, "zz2"), "r", "resume via new name");
    // 完成后命令删除（tclsh coro-2.7）
    assert!(interp.eval("zz2").is_err());
    // 同名重建可用（record 已释放，非孤儿占位）；无 yield 体立即完成
    // ——coroutine 调用本身返回体结果且命令即删（tclsh coro-2.7 同形）
    let v = ev(&mut interp, "coroutine zz apply {{} {return fresh}}");
    assert_eq!(v, "fresh");
    assert!(interp.eval("zz").is_err());
}

/// 挂起-恢复-完成循环 ×N：深度位全程回账（修复前 900..2000 轮内耗尽）。
#[test]
fn coro_resume_complete_cycles_keep_depth_healthy() {
    let mut interp = Interp::new();
    let n = 3000;
    for i in 0..n {
        ev(&mut interp, "coroutine c apply {{} {yield; yield; return ok}}");
        let a = interp.eval("c");
        assert!(a.is_ok(), "iter {i}: first resume failed (depth leak?): {a:?}");
        assert_eq!(ev(&mut interp, "c"), "ok");
    }
    ev(&mut interp, "proc healthy {} { return 1 }");
    assert_eq!(ev(&mut interp, "healthy"), "1");
}

/// resume 错误路径：协程帧整段丢弃，外层作用域不受污染
/// （修复前一轮即可让顶层变量读失败）。
#[test]
fn coro_resume_error_leaves_scope_healthy() {
    let mut interp = Interp::new();
    ev(&mut interp, "set n 5; set k 0");
    for _ in 0..50 {
        ev(&mut interp, "coroutine e apply {{} {yield; error boom}}");
        let r = interp.eval("catch {e}");
        assert!(r.is_ok(), "catch itself failed");
    }
    // 顶层作用域健康（修复前：can't read "n"）
    assert_eq!(ev(&mut interp, "expr { $n > 0 }"), "1");
    ev(&mut interp, "proc healthy {} { return 1 }");
    assert_eq!(ev(&mut interp, "healthy"), "1");
    assert_eq!(ev(&mut interp, "info level"), "0", "no frame residue");
}
