//! fast/classic 共享宿主层（Lane A P2——自 exec.rs 迁出，纯移动）。
//!
//! `Host` = ABI v2 宿主实现（引擎无关方法全集：变量/算术/foreach/
//! 错误帧协议/arena），wasmi 与 wasmtime 的 classic 与 fast 驱动共用。
//! 路径稳定：exec.rs `pub use crate::host::Host` 对外不变。

// 本文件 Host 持 *mut Interp（M2 seam 的所有权交换）；unsafe 收敛于 irp()。
#![allow(unsafe_code)]

use crate::abi::{self, code, BinOp};
use crate::emit;
use crate::ir::FailInfo;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use rtcl_parser::ByteCode;
use rtcl_vm::VmContext;
use std::rc::Rc;

pub(crate) struct Pending {
    pub(crate) err: Error,
    pub(crate) from_eval: bool,
}

pub(crate) struct Host {
    /// 单线程作用域指针：Store 活着期间调用方的 `&mut Interp` 借用
    /// 有效（钩子同步执行，返回后 Host 即弃）。
    pub(crate) interp: *mut Interp,
    pub(crate) arena: Vec<Value>,
    /// Arena length at construction (preloaded constants): fast-tier calls
    /// truncate back to it after every top-level run, so host-tagged
    /// temporaries do not accumulate in the persistent fast Store.
    pub(crate) arena_base: usize,
    pub(crate) consts: Vec<String>,
    /// locals 名（slot 重写的伪常量槽——名字解析同 consts 公式延伸）。
    pub(crate) local_names: Vec<String>,
    pub(crate) fails: Vec<FailInfo>,
    sites: Vec<rtcl_parser::SrcSpan>,
    lines: Vec<u32>,
    source: Rc<str>,
    pub(crate) result: Option<Value>,
    pub(crate) pending: Option<Pending>,
    pub(crate) pending_exit: Option<i32>,
    /// case b 的驱动侧 errorInfo 累积器（机械复刻 err_harness_frame）。
    info: Option<String>,
    err_line: usize,
    pending_top: Option<String>,
    /// 步数预算的 charge 粒度（REFUEL_CHUNK 次 backedge 计 1 步/次）。
    refuel_chunk: i64,
    /// foreach/lmap 宿主态栈（lists/collected）。
    pub(crate) foreaches: Vec<ForeachHost>,
    /// ByteCode 的 foreach 形状表（start/bind 目标用）。
    pub(crate) infos: Vec<rtcl_parser::bytecode::ForeachInfo>,
    /// proc 语境（proc_exec 置位）：case-b 帧写入 interp 累积器
    /// （经 rtcl-vm jit_err_* 公共入口）而非驱动侧字符串——
    /// (procedure …) 边界帧依赖 interp 累积器。
    pub(crate) proc_ctx: bool,
    /// 最近 rtcl.fail 的表索引（case-a 帧构建用）。
    pub(crate) last_fail: Option<u32>,
    /// 入口纪元（④ 中途 bump 检测的比对基准；proc 池 key 含 epoch，
    /// 池命中隐含相等）。
    pub(crate) epoch_at_entry: u64,
    /// ④ 中途 bump 的边界 site（bump 命令自身——重分派从其下一条
    /// 顶层命令起）。
    pub(crate) stale_site: Option<u32>,
    /// 自首次任意宿主命令求值后置位——此后 var_traces/dead_flat 可能
    /// 非空（take_var_fast 的守卫集），LappendVar 等保守路径据此分支。
    pub(crate) commands_ran: bool,
    /// 池复用时的 global 重置句柄（register 时存入）。
    pub(crate) g_fuel_h: Option<wasmi::Global>,
    pub(crate) g_epoch_h: Option<wasmi::Global>,
    pub(crate) g_caching_h: Option<wasmi::Global>,
}

impl Host {
    /// 指针解引用单点（安全性论证见字段 doc：单线程同步作用域）。
    #[allow(clippy::mut_from_ref)]
    fn irp(&self) -> &mut Interp {
        #[allow(unsafe_code)]
        unsafe {
            &mut *self.interp
        }
    }
}

/// 一个活动 foreach/lmap 的宿主态。
pub(crate) struct ForeachHost {
    pub lists: Vec<Vec<Value>>,
    pub collected: Option<Vec<Value>>,
}

/// err_is_error 复刻（控制流完成不参与帧累积）。
fn is_real_error(e: &Error) -> bool {
    !(e.is_return() || e.is_break() || e.is_continue() || e.is_exit() || e.is_tail_call())
}

impl Host {
    pub(crate) fn value_of(&self, tagged: i64) -> Value {
        if tagged & 1 == 1 {
            return Value::from_imm(abi::untag(tagged));
        }
        let idx = (tagged >> 1) as usize;
        match self.arena.get(idx) {
            Some(v) => {
                let c = v.clone();
                // UNSET（bits 0）不得出 arena——合法流里不会出现；出现
                // 即驱动寄存器纪律被破坏（f012451：bitnot 慢路径拿到
                // 未初始化寄存器）——按空值兜底防 SEGV（空指针 deref），
                // 根因（写丢失的精确定位）见 findings 注释。
                if c.is_unset() {
                    return Value::empty();
                }
                c
            }
            None => Value::empty(),
        }
    }

    pub(crate) fn tag_value(&mut self, v: &Value) -> i64 {
        if v.is_imm() {
            return abi::tag_imm(v.imm());
        }
        // arena[0..3] 是保留槽；值从 watermark 起。
        let idx = self.arena.len();
        self.arena.push(v.clone());
        abi::handle(idx as u32)
    }

    fn site_text(&self, site: u32) -> &str {
        self.sites
            .get(site as usize)
            .map(|s| s.slice(&self.source))
            .unwrap_or("")
    }
}

const HARNESS_MAX: usize = 150;

/// tcl_log_excerpt 的逐字节复刻（eval.rs:14）。
fn log_excerpt(text: &str) -> String {
    let len = text.chars().count();
    let cap = len.clamp(100, 150);
    if len > cap {
        let mut s: String = text.chars().take(cap).collect();
        s.push_str("...");
        s
    } else {
        text.to_string()
    }
}

impl Host {
    /// err_harness_frame 复刻（case b；fresh 恒 false——驱动语境）。
    fn harness_frame(&mut self, msg: &str, text: &str, line: usize) {
        match &mut self.info {
            Some(info) => {
                info.push_str(&format!(
                    "\n    invoked from within\n\"{}\"",
                    log_excerpt(text)
                ));
                self.err_line = line;
            }
            None => {
                self.info = Some(format!(
                    "{}\n    while executing\n\"{}\"",
                    msg,
                    log_excerpt(text)
                ));
                self.err_line = line;
            }
        }
        self.pending_top = None;
    }

    /// err_exit_frame 复刻：`\n    (tag line N)`（累积器为空则无操作）。
    fn exit_frame(&mut self, tag: &str) {
        if let Some(info) = &mut self.info {
            info.push_str(&format!("\n    ({} line {})", tag, self.err_line));
        }
    }

    /// unwind_subs 复刻（Sub → defer 最外层；Expr → append；
    /// Body/Next → 无）。
    fn unwind(&mut self, bodies: &[(crate::ir::BodyKind, u32)], e: &Error) {
        // vm_exec unwind_subs 是 **LIFO**（st.bodies.pop()——栈顶先弹）：
        // Sub 先于 BodyFramed（pending_top 随后被 BodyFramed 的
        // harness_frame 清除——外层帧吞掉词括号的 defer）；FIFO 顺序会
        // 反过来让 pending_top 存活到终局、exit_frame 行号取错源
        // （fuzz f000200 簇：尾部多 puts 帧 + ("foreach" body) 行差 1）。
        for (kind, site) in bodies.iter().rev().map(|(k, s)| (*k, *s)) {
            let text = self.site_text(site).to_string();
            let line = self.lines.get(site as usize).copied().unwrap_or(0) as usize;
            match kind {
                crate::ir::BodyKind::Sub => {
                    self.pending_top = Some(text);
                    self.err_line = line;
                }
                crate::ir::BodyKind::Expr => {
                    let msg = e.message_text();
                    self.harness_frame(&msg, &text, line);
                }
                crate::ir::BodyKind::BodyFramed(lmap) => {
                    // vm_exec unwind_subs 的 BodyFramed 复刻：
                    // ("foreach"/"lmap" body line N) + 站点 harness 帧。
                    let tag = if lmap { "\"lmap\" body" } else { "\"foreach\" body" };
                    self.exit_frame(tag);
                    let msg = e.message_text();
                    self.harness_frame(&msg, &text, line);
                }
                crate::ir::BodyKind::Body | crate::ir::BodyKind::Next => {}
            }
        }
    }

    /// case b 的 fail 处置：分类 + （错误时）帧追加。返回 kind。
    pub(crate) fn take_fail(&mut self, fail_id: u32) -> i32 {
        let Some(p) = self.pending.take() else { return 0 };
        let e = p.err;
        // ④ 中途 bump 标记：kind 6（CallSite 模板 → return STALE，
        // 驱动侧重分派）。无帧处理——bump 命令已正常完成。
        if e.message_text() == STALE_MARK {
            return 6;
        }
        if p.from_eval {
            // case a 的行号修补：EvalScript（词括号 script）的树走查种子
            // 只写 err_info 头，err_line 未更新——exit_frame 会打出 stale
            // 行号（rp3：line 1 应为 2）。取 fail 站点行（Sub 外围同规则）。
            if let Some(fi) = self.fails.get(fail_id as usize) {
                if fi.site != u32::MAX {
                    let mut line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                    if let Some((crate::ir::BodyKind::Sub, psite)) = fi.bodies.last() {
                        if *psite != fi.site {
                            line = self.lines.get(*psite as usize).copied().unwrap_or(0) as usize;
                        }
                    }
                    if line > 0 {
                        self.irp().jit_err_set_line(line);
                    }
                }
            }
            // case a：解释器已按协议累积；先分类——控制流穿越
            // （宿主 eval 里的 break/continue 冒出，PR#2 起 `if $c
            // continue` 走 DynCall 的树遍历 cmd_if，不捕获控制流）必须
            // 以 kind 返回，CallSite 的出口分派才能内跳；纯错误保留
            // pending 给终局报告（帧已在 interp，驱动不补）。
            // kind 5：Next 区域的 continue 逃逸（loop_signal 语义——
            // fail 表的 esc 已在快照期解析好落点）。
            let kind = classify_control(&e);
            if kind == 2
                && self
                    .fails
                    .get(fail_id as usize)
                    .is_some_and(|f| f.esc.is_some())
            {
                return 5;
            }
            if kind != 0 {
                return kind;
            }
            self.pending = Some(Pending { err: e, from_eval: true });
            return 0;
        }
        let e = match classify_control(&e) {
            0 => e,
            // kind 5：Next 区域的 continue 逃逸（同 case-a 口径）。
            2 if self
                .fails
                .get(fail_id as usize)
                .is_some_and(|f| f.esc.is_some()) =>
            {
                return 5
            }
            k => return k,
        };
        let fi = self.fails.get(fail_id as usize).cloned();
        if self.proc_ctx {
            // 内联括号形（rd3/dp2 族）：EvalExpr 失败于 Sub region 内、
            // 站点是外围命令（set/puts）——vm 对应形状（内联 StoreVar/
            // 分派后不落该命令帧）不追加站点帧；take_fail 追加即多帧
            // （dp2 多出 "set m […]" 帧）。头帧由 eval_expr 的树走查
            // 种子或内层分派补。
            // proc 语境：帧直写 interp 累积器（call_proc 的 (procedure …)
            // 边界读它）；驱动侧字符串路径不适用。**不抽累积器**：
            // 嵌套 proc 的错误帧已在 interp 上（dispatch_values →
            // call_proc(inner) 已追加），哨兵 error_report 会把它 take
            // 进 self.info——proc_ctx 下那是死存储，帧就此丢失
            // （p06 嵌套差分：inner 的 while-executing/procedure 帧被
            // `"inner"` 站点帧顶替）。jit_err_* 在原累积器上追加，
            // Some→invoked-from-within、None→首帧，与 vm_exec 的
            // err_harness_frame 同构。
            if let Some(fi) = &fi {
                let frameless = fi
                    .bodies
                    .last()
                    .is_some_and(|(_, s)| *s == fi.site);
                if fi.site != u32::MAX && !frameless {
                    let text = self.site_text(fi.site).to_string();
                    // 括号内子命令的 site.line 是**括号相对**行（rp2 实证：
                    // lines_vec=[2,1,3]——expr 站点 1 而 oracle 2）——帧行号
                    // 取外围 Sub 站点的绝对行。
                    let mut line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                    if let Some((crate::ir::BodyKind::Sub, psite)) = fi.bodies.last() {
                        if *psite != fi.site {
                            line = self.lines.get(*psite as usize).copied().unwrap_or(0) as usize;
                        }
                    }
                    let msg = e.message_text();
                    self.irp().jit_err_set_line(line);
                    self.irp().jit_err_harness_frame(&msg, &text, line);
                }
                // 词括号 defer 的 superseded 规则（eval.rs:279）：本层
                // harness 已 append → 其后 Sub region 的 pending defer
                // 帧清除（外层胜出），不再于 unwind 循环里二次落帧
                // （dp2 的 "set m […]" 双帧实证——unwind 的 Sub 臂在
                // ctx-append 之后又 set_pending 一次，最终 error_report
                // 把它追加）。
                let appended_here = fi.site != u32::MAX && !frameless;
                for (kind, site) in fi.bodies.iter().rev() {
                    let text = self.site_text(*site).to_string();
                    let line = self.lines.get(*site as usize).copied().unwrap_or(0) as usize;
                    match kind {
                        crate::ir::BodyKind::Sub => {
                            // 本层已 harness append → Sub defer 帧 superseded
                            // （vm eval.rs:279 同款；dp2 双帧实证）。
                            if !appended_here {
                                self.irp().jit_err_set_pending(&text, line);
                            }
                        }
                        crate::ir::BodyKind::Expr => {
                            let msg = e.message_text();
                            self.irp().jit_err_harness_frame(&msg, &text, line);
                        }
                        crate::ir::BodyKind::BodyFramed(lmap) => {
                            let tag = if *lmap { "\"lmap\" body" } else { "\"foreach\" body" };
                            self.irp().jit_err_exit_frame(tag);
                            let msg = e.message_text();
                            self.irp().jit_err_harness_frame(&msg, &text, line);
                        }
                        crate::ir::BodyKind::Body | crate::ir::BodyKind::Next => {}
                    }
                }
            }
            self.pending = Some(Pending { err: e, from_eval: false });
            return 0;
        }
        // 字符串路径的基底：宿主原语错误也可能已在 interp 累积帧（如
        // trace 脚本失败）——哨兵取出作为基底（无累积器时 = 消息文本，
        // 两种情形同构）。**基底抽取前先清 interp 的 pending defer**：
        // 本层即将 append 站点帧（下方的 harness_frame），vm 语义里
        // harness append 会 supersede 掉 defer 帧（eval.rs:279）——
        // error_report 的 flush 会把 defer 帧提前落进基底（dp6/f000319：
        // "set m […]" 出现在站点帧之前的根因）。
        if let Some(fi) = &fi {
            let frameless = fi
                .bodies
                .last()
                .is_some_and(|(_, s2)| *s2 == fi.site);
            if fi.site != u32::MAX && !frameless {
                self.irp().jit_err_clear_pending();
            }
        }
        {
            const SENT: &str = "\u{0}j";
            let raw = self.irp().error_report(&e, SENT);
            let tail = format!("\n    (file \"{}\" line ", SENT);
            let base = match raw.rfind(&tail) {
                Some(i) => Some(raw[..i].to_string()),
                None if raw != e.to_string() => Some(raw),
                None => None,
            };
            if let Some(b) = base {
                self.info = Some(b);
            }
        }
        if let Some(fi) = fi {
            let frameless = fi
                .bodies
                .last()
                .is_some_and(|(_, s)| *s == fi.site);
            if fi.site != u32::MAX && !frameless {
                let text = self.site_text(fi.site).to_string();
                let line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                let msg = e.message_text();
                self.harness_frame(&msg, &text, line);
            }
            self.unwind(&fi.bodies, &e);
        }
        self.pending = Some(Pending { err: e, from_eval: false });
        0
    }

    /// case-a 帧构建：interp 累积器（经哨兵标签 error_report 取出——
    /// 空 file 名不可能出现在真实路径，剥尾确定）+ 驱动补帧（EvalExpr
    /// 站点 harness / EvalScript pending / unwind 的 Sub/Expr/BodyFramed）
    /// + 文件帧（err_line 取最后一次 append 的行）。
    pub(crate) fn build_case_a_report(&mut self, fail_id: Option<u32>, e: &Error, file: &str) -> String {
        const SENT: &str = "\u{0}j";
        let raw = self.irp().error_report(e, SENT);
        let tail = format!("\n    (file \"{}\" line ", SENT);
        let base = match raw.rfind(&tail) {
            Some(i) => raw[..i].to_string(),
            None => raw,
        };
        // base == 裸消息（e.to_string()）⇒ interp 累积器为空（无任何
        // 帧 append 过）——保持 info=None，让驱动补帧走 harness_frame
        // 的**首帧**分支（"msg\n    while executing\n…"）；把裸消息
        // 塞进 info 会让补帧走 append 分支丢帧头（fuzz f001026：
        // `puts [expr {$n && $s}]` 的 expr 站点帧）。
        self.info = if !base.is_empty() && base != e.to_string() {
            Some(base)
        } else {
            None
        };
        let mut pending_frame: Option<(String, usize)> = None;
        if let Some(fid) = fail_id {
            if let Some(fi) = self.fails.get(fid as usize).cloned() {
                let text = self.site_text(fi.site).to_string();
                let line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                // err_line 初始化 = 最内层失败帧的行（vm_exec 的 err_line
                // 由嵌套 eval 的最后一次 append 设置；此处以 fail 站点行
                // 近似——exit_frame 在 harness append 之前读取它）。
                if self.info.is_some() {
                    self.err_line = line;
                }
                match fi.kind {
                    crate::ir::FailKind::EvalExpr => {
                        let msg = e.message_text();
                        self.harness_frame(&msg, &text, line);
                    }
                    crate::ir::FailKind::EvalScript => {
                        pending_frame = Some((text, line));
                    }
                    _ => {}
                }
                self.unwind(&fi.bodies, e);
            }
        }
        // pending 帧最终追加（vm_exec error_report 的 drain 语义）：
        // unwind 的 Sub 覆盖 step-1 的 word pending（外层胜出）。
        // pending_top 只带文本（无行——unwind 已设置 err_line）。
        let mut pending_final = pending_frame;
        if let Some(t) = self.pending_top.take() {
            let l = self.err_line;
            pending_final = Some((t, l));
        }
        if let Some((t, l)) = pending_final {
            if let Some(info) = &mut self.info {
                info.push_str(&format!(
                    "\n    invoked from within\n\"{}\"",
                    log_excerpt(&t)
                ));
                self.err_line = l;
            }
        }
        if let Some(info) = &mut self.info {
            info.push_str(&format!("\n    (file \"{}\" line {})", file, self.err_line));
        }
        self.info.take().unwrap_or_else(|| e.to_string())
    }

    /// error_report 复刻（case b 终局）。
    pub(crate) fn driver_report(&mut self, file: &str) -> String {
        let e = self.pending.as_ref().map(|p| p.err.clone());
        match e {
            Some(e) if is_real_error(&e) => {
                if let Some(text) = self.pending_top.take() {
                    if let Some(info) = &mut self.info {
                        info.push_str(&format!(
                            "\n    invoked from within\n\"{}\"",
                            log_excerpt(&text)
                        ));
                    }
                }
                let tag = format!("file \"{}\"", file);
                if let Some(info) = &mut self.info {
                    info.push_str(&format!("\n    ({} line {})", tag, self.err_line));
                }
                self.info.take().unwrap_or_else(|| e.to_string())
            }
            Some(e) => e.to_string(),
            None => String::new(),
        }
    }
}

/// ④ 中途 bump 的 pending 标记（take_fail → kind 6）。
pub(crate) const STALE_MARK: &str = "\u{0}jit-stale";

/// 控制流分类（fail 的返回 kind）：0=错误、1/3=break（内层/外层）、
/// 2/4=continue。level≤1 = 属于本单元的内层循环。
pub(crate) fn classify_control(e: &Error) -> i32 {
    if e.is_break() {
        if e.loop_level() > 1 { 3 } else { 1 }
    } else if e.is_continue() {
        if e.loop_level() > 1 { 4 } else { 2 }
    } else {
        0
    }
}

/// expr 操作数的 canonical 引用（braced 优先，双引号转义兜底）。
fn expr_quote(s: &str) -> String {
    // 数值字面量裸写——expr 里 {} 是**字符串字面量**，数值操作数
    // braced 会让算术报 "can't use non-numeric string"（fuzz f002484：
    // Neg 慢路径的 -2^62 → bignum 宽化）。仅数值形态裸写（数字开头，
    // 含 dec/hex/oct/bin/科学计数/小数）；标识符/布尔词裸写会踩 expr
    // 的 bareword 门槛（"abc + 1" 报 invalid bareword 而非
    // non-numeric——错误文本走样），维持 braced（字符串语义）。
    let numeric = {
        let body = s.strip_prefix(['+', '-']).unwrap_or(s);
        !body.is_empty()
            && body.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && body.chars().all(|c| {
                c.is_ascii_hexdigit()
                    || matches!(c, '.' | 'x' | 'X' | 'o' | 'O' | 'b' | 'B' | 'e' | 'E' | '+' | '-')
            })
    };
    if numeric {
        return s.to_string();
    }
    let braced_ok = {
        let mut depth: i32 = 0;
        let mut ok = true;
        for c in s.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        ok = false;
                        break;
                    }
                }
                '\\' | '[' | '$' | '"' | ';' => {}
                _ => {}
            }
        }
        ok && depth == 0 && !s.ends_with('\\')
    };
    if braced_ok {
        format!("{{{}}}", s)
    } else {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '\\' | '"' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// arena 预驻留：0=UNSET 1=哨兵保留 2=EMPTY 3..=常量池，随后 =
/// locals 名（slot 重写的伪常量槽——alloc_locals 同公式）。
pub(crate) fn arena_of(bc: &ByteCode) -> Vec<Value> {
    let mut arena = vec![Value::unset(), Value::unset(), Value::empty()];
    for c in bc.constants() {
        arena.push(Value::from_str(c));
    }
    for l in bc.locals() {
        arena.push(Value::from_str(l));
    }
    arena
}

impl Host {
    // ── 构造与引擎协作 ──────────────────────────────────────────────

    pub fn new(interp: *mut Interp, arena: Vec<Value>, bc: &ByteCode, module: &emit::Module) -> Host {
        Host {
            interp,
            arena_base: arena.len(),
            arena,
            consts: bc.constants().to_vec(),
            local_names: bc.locals().to_vec(),
            fails: module.l2.fails.clone(),
            sites: bc.sites.iter().map(|s| s.text).collect(),
            lines: bc.sites.iter().map(|s| s.line).collect(),
            source: Rc::clone(&bc.source),
            result: None,
            pending: None,
            pending_exit: None,
            info: None,
            err_line: 0,
            pending_top: None,
            refuel_chunk: abi::REFUEL_CHUNK,
            foreaches: Vec::new(),
            infos: {
                let mut v = Vec::new();
                let mut i = 0u32;
                while let Some(f) = bc.foreach_info(i) {
                    v.push(f.clone());
                    i += 1;
                }
                v
            },
            last_fail: None,
            proc_ctx: false,
            commands_ran: false,
            g_fuel_h: None,
            g_epoch_h: None,
            g_caching_h: None,
            epoch_at_entry: unsafe { (*interp).tier1_epoch() },
            stale_site: None,
        }
    }

    pub fn arena_len(&self) -> usize {
        self.arena.len()
    }

    /// 实例池复用前的状态复位（arena 已在 run 后截回水位线；fuel/
    /// epoch/caching global 持续——语义一致）。
    pub fn reset_for_reuse(&mut self) {
        self.result = None;
        self.pending = None;
        self.pending_exit = None;
        self.info = None;
        self.pending_top = None;
        self.last_fail = None;
        self.stale_site = None;
        self.foreaches.clear();
        self.err_line = 0;
    }

    pub fn arena_truncate(&mut self, n: usize) {
        self.arena.truncate(n);
    }

    pub(crate) fn site_text_of(&self, site: u32) -> String {
        self.site_text(site).to_string()
    }
    pub(crate) fn lines_of(&self, site: u32) -> usize {
        self.lines.get(site as usize).copied().unwrap_or(0) as usize
    }
    pub fn refuel_chunk(&self) -> i64 {
        self.refuel_chunk
    }

    pub fn interp_mut(&mut self) -> &mut Interp {
        unsafe { &mut *self.interp }
    }

    pub fn set_pending(&mut self, err: Error, from_eval: bool) {
        self.pending = Some(Pending { err, from_eval });
    }

    // ── 宿主逻辑（引擎无关；各引擎的 import 包装只做参数搬运）──────

    pub fn binary_op_raw(&mut self, op: i32, a: i64, b: i64) -> i64 {
        
        self.do_binary_op(op, a, b)
    }

    fn do_binary_op(&mut self, op: i32, a: i64, b: i64) -> i64 {
        let r = self.binary_op_impl(op, a, b);
        // 布尔值域 op（比较/逻辑）归一为 imm 0/1——vm_exec 的 op_eq/
        // op_rel 直接 from_bool（imm）；eval_expr 路由可能产出字符串
        // 形态（"0"/"1"），下游 BoolOfBool/BoolTest 的 tagged-0/1 假设
        // 依赖 imm 形状。
        const BOOL_OPS: &[i32] = &[
            14, 15, 16, 17, 18, 19, 20, 21, 22, 23, // Eq Ne Lt Gt Le Ge StrEq StrNe And Or
        ];
        if BOOL_OPS.contains(&op) {
            if let Ok(v) = self.value_of_result(r) {
                if let Some(i) = v.as_int() {
                    return abi::tag_imm(i.clamp(0, 1));
                }
            }
        }
        r
    }

    fn value_of_result(&self, tagged: i64) -> Result<Value, ()> {
        if tagged & 1 == 1 {
            return Ok(Value::from_imm(abi::untag(tagged)));
        }
        self.arena
            .get((tagged >> 1) as usize)
            .cloned()
            .ok_or(())
    }

    fn binary_op_impl(&mut self, op: i32, a: i64, b: i64) -> i64 {
        match self.binary_op(op, a, b) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn get_var_raw(&mut self, name_h: i32) -> i64 {
        let name = self.const_name(name_h);
        // ⑥ 数组下标感知：名字可携带未替换下标（expr 编译放行
        // $name(index) 后，LoadVar 的名字是原文）——jit_read_var_ref
        // 做下标替换；裸 read_var 会按字面 "$k" 找元素。
        match self.irp().jit_read_var_ref(&name) {
            Ok(v) => {
                
                self.tag_value(&v)
            }
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn set_var_raw(&mut self, name_h: i32, v: i64) -> i32 {
        let name = self.const_name(name_h);
        let val = self.value_of(v);
        match self.irp().set_var(&name, val) {
            Ok(_) => 0,
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                1
            }
        }
    }

    pub fn incr_var_raw(&mut self, name_h: i32, n: i64) -> i64 {
        let name = self.const_name(name_h);
        match self.irp().incr_var(&name, abi::untag(n)) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    /// 命令分派：站点原文 + 换行前缀 eval 路由。
    pub fn call_raw(&mut self, _cmd_id: i32, site: i32, _argc: i32) -> i64 {
        self.commands_ran = true;
        let site = site.max(0) as u32;
        let text = self.site_text(site).to_string();
        let line = self.lines.get(site as usize).copied().unwrap_or(1).max(1) as usize;
        let script = format!("{}{}", "\n".repeat(line - 1), text);
        match self.irp().eval(&script) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: true });
                abi::ERR_SENTINEL
            }
        }
    }

    /// 直连分派（M2 全案）：已求值实参 → charge_step → dispatch_values。
    /// 错误 = case-b（from_eval=false）——驱动补站点帧；无宿主 eval 记账。
    pub fn call_argv_raw(&mut self, words: &[u64], site: i32) -> i64 {
        let mut args: Vec<Value> = Vec::with_capacity(words.len());
        for w in words {
            args.push(self.value_of(*w as i64));
        }
        if let Err(e) = rtcl_vm::VmContext::charge_step(self.irp()) {
            self.pending = Some(Pending { err: e, from_eval: false });
            return abi::ERR_SENTINEL;
        }
        // 直呼快路径：cmd_cache 命中 proc 且形状门过 → 免 dispatch
        // 臂层直达 call_proc（fib 差分剖析：dispatch 层叠 ~30-50ns/调用）。
        let r = match self
            .irp()
            .jit_call_proc_direct(args[0].as_str(), &args)
        {
            Some(r) => r,
            None => self.irp().dispatch_values(&args),
        };
        match r {
            Ok(v) => {
                // ④ Tier1 中途 bump：当前命令已完成（结果暂存——bump
                // 恰在末条命令时即单元结果），从其下一条顶层命令重分派。
                // 经既有哨兵→fail→kind 协议：kind 6 → wasm return STALE，
                // 驱动侧调 jit_redispatch_after。
                if self.irp().tier1_epoch() != self.epoch_at_entry {
                    self.stale_site = Some(site.max(0) as u32);
                    self.result = Some(v);
                    self.pending = Some(Pending {
                        err: Error::Msg(STALE_MARK.to_string()),
                        from_eval: false,
                    });
                    return abi::ERR_SENTINEL;
                }
                self.tag_value(&v)
            }
            Err(e) => {
                // case-b：帧由 wasm 侧哨兵 → rtcl.fail(fail_id) →
                // take_fail 用 fail 表的 site 补（vm_exec exec_inner 同款）。
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    /// v2.1 槽重同步：名字读或 0（未定义）——无哨兵协议。
    pub fn sync_var_raw(&mut self, name_h: i32) -> i64 {
        let name = self.const_name(name_h);
        let r = self.irp().get_var(&name).cloned();
        match r {
            Ok(v) => self.tag_value(&v),
            Err(_) => 0,
        }
    }

    pub fn set_result_raw(&mut self, v: i64) {
        let val = self.value_of(v);
        self.result = Some(val);
    }

    pub fn bool_test_raw(&mut self, v: i64) -> i64 {
        match self.bool_test(v) {
            Ok(b) => abi::tag_imm(b as i64),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn unbox_int_raw(&self, h: i32) -> (i32, i64) {
        let v = self.arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
        match v.as_int() {
            Some(n) => (1, n),
            None => (0, 0),
        }
    }

    pub fn unbox_float_raw(&self, h: i32) -> (i32, f64) {
        let v = self.arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
        match v.as_float() {
            Some(f) => (1, f),
            None => (0, 0.0),
        }
    }

    pub fn box_raw(&mut self, v: i64) -> i32 {
        let val = self.value_of(v);
        let idx = self.arena.len();
        self.arena.push(val);
        ((idx as u64) << 1) as i32
    }

    pub fn box_float_raw(&mut self, f: f64) -> i32 {
        let val = Value::from_float(f);
        let idx = self.arena.len();
        self.arena.push(val);
        ((idx as u64) << 1) as i32
    }

    /// `rtcl.concat_n(base_off, n)`：从模块 memory 读 n 个 tagged word，
    /// as_str 连接（vm_exec Concat 语义——永不失败）。
    pub fn concat_raw(&mut self, words: &[u64]) -> i64 {
        let mut joined = String::new();
        for w in words {
            let v = self.value_of(*w as i64);
            joined.push_str(v.as_str());
        }
        let out = Value::from_str(&joined);
        self.tag_value(&out)
    }

    pub fn fail_raw(&mut self, fail_id: i32) -> i32 {
        self.last_fail = Some(fail_id.max(0) as u32);
        self.take_fail(fail_id.max(0) as u32)
    }

    /// `foreach_start(info, words)`：strict 解析 + bind(0)；返回 tagged
    /// iters 或哨兵。
    pub fn foreach_start_raw(&mut self, info: i32, words: &[u64]) -> i64 {
        let info = info.max(0) as usize;
        let Some(fi) = self.infos.get(info).cloned() else {
            return abi::tag_imm(0);
        };
        // vm_exec：逆序弹组后 lists.reverse() → 最终 lists[0]=组0。
        // 暂存区升序（srcs = base..depth = 栈序）正读即同序——不再逆读
        // （fuzz f002609：多组 foreach 组间错位）。
        let mut lists = Vec::with_capacity(words.len());
        for w in words.iter() {
            let v = self.value_of(*w as i64);
            match v.as_list_ref() {
                Some(items) => lists.push(items.to_vec()),
                None => match v.as_list_strict() {
                    Ok(items) => lists.push(items),
                    Err(e) => {
                        let _ = self.irp().set_var("errorCode", Value::from_str(e.code));
                        self.pending = Some(Pending {
                            err: Error::Msg(e.message),
                            from_eval: false,
                        });
                        return abi::ERR_SENTINEL;
                    }
                },
            }
        }
        let max = fi
            .groups
            .iter()
            .zip(&lists)
            .map(|(g, l)| l.len().div_ceil(g.len().max(1)))
            .max()
            .unwrap_or(0);
        self.foreaches.push(ForeachHost {
            lists,
            collected: fi.lmap.then(Vec::new),
        });
        if max > 0 && self.foreach_bind_idx(&fi, 0).is_err() {
            return abi::ERR_SENTINEL;
        }
        abi::tag_imm(max as i64)
    }

    /// bind 第 idx 次迭代的循环变量（foreach_bind 复刻：空值兜底、
    /// framed 装饰、TCL WRITE VARNAME）。
    fn foreach_bind_idx(
        &mut self,
        fi: &rtcl_parser::bytecode::ForeachInfo,
        idx: usize,
    ) -> Result<(), ()> {
        let empty = Value::empty();
        let groups = fi.groups.clone();
        for (g_i, g) in groups.iter().enumerate() {
            let n = g.len();
            for (vi, t) in g.iter().enumerate() {
                let v = self
                    .foreaches
                    .last()
                    .and_then(|f| f.lists.get(g_i))
                    .and_then(|l| l.get(idx * n + vi))
                    .cloned()
                    .unwrap_or_else(|| empty.clone());
                let name = self.consts.get(t.name_idx as usize).cloned().unwrap_or_default();
                if let Err(e) = self.irp().set_var(&name, v) {
                    if is_real_error(&e) {
                        let _ = self
                            .irp()
                            .set_var("errorCode", Value::from_str("TCL WRITE VARNAME"));
                        if fi.framed {
                            let what = if fi.lmap { "lmap" } else { "foreach" };
                            if self.info.is_none() {
                                self.info = Some(e.message_text());
                            }
                            let deco = format!("\n    (setting {} loop variable \"{}\")", what, name);
                            if let Some(is) = &mut self.info {
                                is.push_str(&deco);
                            }
                        }
                    }
                    self.pending = Some(Pending { err: e, from_eval: false });
                    return Err(());
                }
            }
        }
        Ok(())
    }

    pub fn foreach_bind_raw(&mut self, info: i32, idx: i32) -> i64 {
        let fi = match self.infos.get(info.max(0) as usize) {
            Some(f) => f.clone(),
            None => return 0,
        };
        match self.foreach_bind_idx(&fi, idx.max(0) as usize) {
            Ok(()) => 0,
            Err(()) => abi::ERR_SENTINEL,
        }
    }

    pub fn foreach_collect_raw(&mut self, v: i64) {
        let val = self.value_of(v);
        if let Some(f) = self.foreaches.last_mut() {
            if let Some(c) = &mut f.collected {
                c.push(val);
            }
        }
    }

    pub fn foreach_end_raw(&mut self) -> i64 {
        let f = self.foreaches.pop();
        let v = match f.and_then(|f| f.collected) {
            Some(c) => Value::from_list(&c),
            None => Value::empty(),
        };
        self.tag_value(&v)
    }

    /// LappendVar 宿主全语义（vm_exec LappendVar 臂的公共 API 复刻）：
    /// 存在且 list rep → 追加；不存在 → 创建 [value]；非 list rep →
    /// strict 错误（cmd_lappend 的形状）。
    pub fn lappend_var_raw(&mut self, name_h: i32, v: i64) -> i64 {
        let name = self.const_name(name_h);
        let val = self.value_of(v);
        if self.irp().var_exists(&name) {
            match self.irp().get_var(&name) {
                Ok(cur) => match cur.as_list() {
                    Some(mut items) => {
                        items.push(val);
                        let nv = Value::from_list(&items);
                        match self.irp().set_var(&name, nv.clone()) {
                            Ok(_) => self.tag_value(&nv),
                            Err(e) => {
                                self.pending = Some(Pending { err: e, from_eval: false });
                                abi::ERR_SENTINEL
                            }
                        }
                    }
                    None => {
                        // 非 list rep：cmd_lappend 的 strict 错误。
                        let s = cur.as_str().to_string();
                        let probe = Value::from_str(&s);
                        match probe.as_list_strict() {
                            Ok(items) => {
                                // as_list 宽松失败但 strict 成功（罕见形状）
                                let mut items = items;
                                items.push(val);
                                let nv = Value::from_list(&items);
                                match self.irp().set_var(&name, nv.clone()) {
                                    Ok(_) => self.tag_value(&nv),
                                    Err(e) => {
                                        self.pending =
                                            Some(Pending { err: e, from_eval: false });
                                        abi::ERR_SENTINEL
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = self
                                    .irp()
                                    .set_var("errorCode", Value::from_str(e.code));
                                self.pending = Some(Pending {
                                    err: Error::Msg(e.message),
                                    from_eval: false,
                                });
                                abi::ERR_SENTINEL
                            }
                        }
                    }
                },
                Err(e) => {
                    self.pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        } else {
            let nv = Value::from_list(&[val]);
            match self.irp().set_var(&name, nv.clone()) {
                Ok(_) => self.tag_value(&nv),
                Err(e) => {
                    self.pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        }
    }

    /// EvalScript 路由：嵌套 eval；错误 from_eval=true（站点 pending 帧
    /// 由 case-a 构建器补）。
    pub fn eval_script_raw(&mut self, v: i64) -> i64 {
        self.commands_ran = true;
        let val = self.value_of(v);
        let script = val.as_str().to_string();
        match self.irp().eval(&script) {
            Ok(r) => self.tag_value(&r),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: true });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn eval_expr_raw(&mut self, v: i64) -> i64 {
        let val = self.value_of(v);
        let expr = val.as_str().to_string();
        match self.irp().eval_expr(&expr) {
            Ok(r) => self.tag_value(&r),
            Err(e) => {
                // 树走查 eval_expr 对**求值期**错误（非解析期）不落
                // 任何帧——纯解释里帧来自分派层 harness；JIT 的对应
                // 物 = take_fail 的 case-b 站点帧（vm:779 同款）。
                // from_eval=true 会走 case-a「解释器已累积」早退——
                // 帧就此丢失（rd3：内层 expr 帧被外层 call 帧顶替）。
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn exit_raw(&mut self, code: i32) {
        self.pending_exit = Some(code);
    }

    pub(crate) fn const_name(&self, name_h: i32) -> String {
        let idx = ((name_h >> 1) as usize).saturating_sub(abi::CONST_ARENA_BASE as usize);
        if idx < self.consts.len() {
            self.consts[idx].clone()
        } else {
            // 伪常量槽：locals 名（slot 重写——arena 预驻留同公式）。
            self.local_names
                .get(idx - self.consts.len())
                .cloned()
                .unwrap_or_default()
        }
    }

    /// binary_op：宿主完整 Tcl 语义（§3）——`types::eval_expr` 原文路由。
    fn binary_op(&mut self, op: i32, a: i64, b: i64) -> Result<Value, Error> {
        // 表达式可含命令替换（eval_expr 求值 [..]）——置位。
        self.commands_ran = true;
        let kind = op;
        let va = self.value_of(a);
        let vb = self.value_of(b);
        let sa = expr_quote(va.as_str());
        let sb = expr_quote(vb.as_str());
        let expr = match kind {
            x if x == BinOp::Add as i32 => format!("{sa} + {sb}"),
            x if x == BinOp::Sub as i32 => format!("{sa} - {sb}"),
            x if x == BinOp::Mul as i32 => format!("{sa} * {sb}"),
            x if x == BinOp::Div as i32 => format!("{sa} / {sb}"),
            x if x == BinOp::Mod as i32 => format!("{sa} % {sb}"),
            x if x == BinOp::Pow as i32 => format!("{sa} ** {sb}"),
            // -(sa)：操作数可能是负数字面量（裸写）——"- -5" 形态的
            // 拼接会破坏括号子表达式的字面量结构（f002484：NEG 慢路径
            // 的 bignum 宽化 "--4611…" 非法词）。
            x if x == BinOp::Neg as i32 => format!("-({sa})"),
            x if x == BinOp::Not as i32 => format!("!{sa}"),
            x if x == BinOp::BitNot as i32 => format!("~{sa}"),
            x if x == BinOp::BitAnd as i32 => format!("{sa} & {sb}"),
            x if x == BinOp::BitOr as i32 => format!("{sa} | {sb}"),
            x if x == BinOp::BitXor as i32 => format!("{sa} ^ {sb}"),
            x if x == BinOp::Shl as i32 => format!("{sa} << {sb}"),
            x if x == BinOp::Shr as i32 => format!("{sa} >> {sb}"),
            x if x == BinOp::Eq as i32 => format!("{sa} == {sb}"),
            x if x == BinOp::Ne as i32 => format!("{sa} != {sb}"),
            x if x == BinOp::Lt as i32 => format!("{sa} < {sb}"),
            x if x == BinOp::Gt as i32 => format!("{sa} > {sb}"),
            x if x == BinOp::Le as i32 => format!("{sa} <= {sb}"),
            x if x == BinOp::Ge as i32 => format!("{sa} >= {sb}"),
            x if x == BinOp::StrEq as i32 => format!("{sa} eq {sb}"),
            x if x == BinOp::StrNe as i32 => format!("{sa} ne {sb}"),
            x if x == BinOp::And as i32 => format!("{sa} && {sb}"),
            x if x == BinOp::Or as i32 => format!("{sa} || {sb}"),
            _ => return Err(Error::Msg("jit: bad binary_op".into())),
        };
        let _ = HARNESS_MAX;
        self.irp().eval_expr(&expr)
    }

    /// strict_bool 复刻（expr_funcs::strict_bool 的公共 API 重实现：
    /// 数值非零真、布尔词及其无歧义前缀，否则 exact 错误文本）。
    fn bool_test(&self, v: i64) -> Result<bool, Error> {
        let val = self.value_of(v);
        if let Some(i) = val.as_int() {
            return Ok(i != 0);
        }
        if let Some(f) = val.as_float() {
            return Ok(f != 0.0);
        }
        let lower = val.as_str().trim().to_ascii_lowercase();
        if !lower.is_empty() {
            let candidates =
                ["true", "false", "yes", "no", "on", "off"].into_iter().filter(|w| w.starts_with(&lower));
            let mut it = candidates;
            let first = it.next();
            if let Some(first) = first {
                if it.next().is_none() {
                    return Ok(matches!(first, "true" | "yes" | "on"));
                }
            }
        }
        Err(Error::Msg(format!(
            "expected boolean value but got \"{}\"",
            val.as_str()
        )))
    }
}
