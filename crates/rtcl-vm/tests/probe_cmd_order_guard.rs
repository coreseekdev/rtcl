//! 注册序日志（command_order）护栏回归（specs/MEMORY-AUDIT-20261120.md
//! 命令 churn 面）：创建+rename away 的孤儿条目此前线性增长
//! （~100B/命令，oracle tclsh 同型零增长）；护栏在条目数 >4096 且
//! >4×活命令时整表重建（保留活条目与注册次序）。行为面断言：churn
//! 后 `info commands` replay 序保持正确、活命令可调、孤儿名不回流。

use rtcl_vm::interp::Interp;

fn ev(i: &mut Interp, s: &str) -> String {
    i.eval(s).unwrap().as_str().to_string()
}

/// 注册序日志护栏：命令 churn（创建+rename away，6000 孤儿 > 4096
/// 阈值）后 `info commands` 输出不受孤儿条目影响、活命令可调。
#[test]
fn command_order_guard_keeps_listing_correct_under_churn() {
    let mut interp = Interp::new();
    for k in 0..6000 {
        ev(&mut interp, &format!("proc q{k} {{x}} {{ return {k} }}"));
        ev(&mut interp, &format!("rename q{k} {{}}"));
    }
    ev(&mut interp, "proc alive {x} { return a-$x }");
    ev(&mut interp, "rename alive zzz");
    let listing = ev(&mut interp, "info commands zzz");
    assert_eq!(listing, "zzz", "renamed-alive command must list once");
    assert_eq!(ev(&mut interp, "zzz 7"), "a-7");
    // 孤儿名不出现（护栏重建 + 活性过滤双保险）
    assert_eq!(ev(&mut interp, "llength [info commands q3*]"), "0");
    // 重建后再建新命令：replay 序尾部追加语义保持
    ev(&mut interp, "proc post {} { return p }");
    assert_eq!(ev(&mut interp, "info commands post"), "post");
}
