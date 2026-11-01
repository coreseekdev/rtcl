# JIT 引擎架构合并计划（Lane A——目标对齐 2026-11-01）

> 用户拍板：双线并行。**JIT 相关部分架构合并**（rtcl 已有部分 JIT 能力，
> 散在多处执行器），**towasm 独立**（产物线，见 crates/rtcl-towasm）。
> 本文档 = Lane A 的盘点、目标布局、分期计划与门禁纪律。

## 1. 现状盘点（8a76420）

| 文件 | 行数 | 职责 | 引擎 |
|---|---|---|---|
| exec.rs | 1673 | Host（引擎无关方法全集）+ classic linker（wasmi）+ 顶层 execute + finish | wasmi |
| proc_exec.rs | 1656 | 钩子（install/exec_proc_body）+ **fast-tier wasmi 全量**（FastHost/fast_linker/fast_ensure_slot/execute_fast/take_fail/rc_to_i64）+ classic 池（execute_proc[_pooled]）+ compile_cached | wasmi |
| exec_fast_wt.rs | 887 | **fast-tier wasmtime 全量**（FastHostWt/…/execute_fast_wt——proc_exec 的逐函数镜像） | wasmtime |
| exec_wasmtime.rs | 350 | classic linker（wasmtime）+ 顶层 execute + register_wasmtime_abi | wasmtime |
| abi.rs | 247 | ABI 常量/编码/布局——**单一真值**（两引擎共享，已合并） | 共享 |
| pass/ | ~4000 | lower/assemble/… 发射管线（引擎无关，天然共享） | 共享 |

**镜像重复清单**（wasmi ↔ wasmtime 逐函数对照，~800 行）：
FastHost{,Wt} / fast_linker{,_wt}（27 闭包）/ fast_ensure_slot{,_wt} /
execute_fast{,_wt} / fast_take_fail{,_wt} / read_cur{,_wt} /
refresh_epoch{,_wt} / fast_read_words{,_wt} / fast_rc_to_i64{,_wt}。
classic linker 同病（register_wasmtime_abi vs exec.rs 内联段）。

**已定谳的引擎 API 不对称**（888ff1e 移植实测——合并的 trait 设计输入）：
1. `Linker::define(store, module, name, item)`（wasmtime 带 store 参）vs `define(module, name, item)`（wasmi）；
2. 泛型上下文取数据：wasmtime `as_context().data()`（`data()` 是 Store 固有方法）vs wasmi `store.data()`；
3. `TypedFunc` 非 Copy：wasmtime 需 `.clone()`，取 Func 用 `.func()`；
4. `Table::set(store, idx, Ref::Func(Some(f.func())))`（wasmtime Ref 包 Option）vs wasmi `Ref::Func(f.into())`；
5. `Caller` 不 Deref：wasmtime 闭包里 `&mut c` 直传（wasmi 同名方法借 self）；
6. wasmi `Instance::get_export(ctx, name)` 两参 vs wasmtime 同两参但 ctx 形态不同；
7. `Import` 段解析（产物校验用）：wasmparser 0.261 `into_imports()` 展平紧凑编码。

## 2. 目标布局

```
crates/rtcl-jit/src/
  abi.rs            # 不动（单一真值）
  pass/             # 不动（发射管线，引擎无关）
  host.rs           # Host 全方法（自 exec.rs 迁出——引擎无关，两线共享）
  engine/
    mod.rs          # EngineOps trait + 引擎选择（engine_from_env）
    classic_wasmi.rs  # exec.rs 的 classic 段
    classic_wt.rs     # exec_wasmtime.rs
    fast_wasmi.rs     # proc_exec.rs 的 fast 段
    fast_wt.rs        # exec_fast_wt.rs
  proc_exec.rs      # 钩子 + compile_cached + fast_only/aggressive 策略（瘦身）
  exec.rs           # run_top/run_top_with/finish + Towasm 消费面（瘦身）
```

**EngineOps trait 草图**（消除镜像的关键——把上述 7 条不对称收进一个
impl 对）：

```rust
pub trait EngineOps: 'static {
    type Store: AsContextMut<Data = Self::Host>;
    type Host; type Global: Copy+Clone; type Memory; type Table;
    type Func; type TypedProcMain;   // (i32,i32,i32)->(i64,i32)
    fn global_new_i32(store, v: i32) -> Global;
    fn global_set_i32(g, store, v: i32);
    fn global_get_i32(g, store) -> i32;
    fn mem_read(store, mem, addr, buf) -> bool;
    fn mem_write(store, mem, addr, bytes) -> bool;
    fn table_set_func(store, tbl, idx, f: &Func);
    fn caller_cur_proc(c: &mut Caller<Self::Host>) -> usize;   // read_cur
    fn caller_dirty(c: &mut Caller<Self::Host>) -> bool;       // dirty 门读
    fn caller_dirty_clear(c: &mut Caller<Self::Host>);
    fn proc_main_call(store, tf, (slot, argc, base)) -> Result<(i64,i32), Trap>;
    fn linker_define_globals(linker, store, & seven-tuple);
}
```

宿主闭包体（逻辑部分：incr_var_raw/set_var_raw/dispatch/哨兵传播）
已 100% 引擎无关（exec::Host 方法）——闭包只差「取上下文 + 调 Host
方法」的胶水，trait 化后 fast_linker 写一遍，两引擎各给一个
EngineOps impl。

## 3. 分期计划（每期门禁 = judge 88/88×2 + battery 33/33 + probes
18/18×2 + fuzz 800×2 + rtcl-vm 809 + bench/cases 六例 ≥ 平价）

- **P3-lite ✅ 已完成**（e1679ac，2026-11-01）：FastShared 共享层 +
  sh_* 逻辑体 10 函数单点化；两引擎闭包瘦身为胶水+委托；~400 行
  重复逻辑消除（1656+887 → 1441+680+314）。语义修复从此单点落地
  （p05/dirty/sync/kind5 系列的双落痛点解除）。EngineOps trait
  （闭包签名级合并）推迟——sh_ 委托已消除逻辑重复，trait 的边际
  收益剩胶水行数，风险/收益不划算时可以停在这里。
- **P1 ✅ 已完成**（73582ea）：fast/ 与 classic/ 模块树成型——
  proc_exec.rs 1656→101（纯钩子层：install + 策略 + 引擎分派）；
  fast/{shared,wasmi,wt} + classic/{mod,wasmi,wt} 各归其位。
  TL 拆分教训：原 51-79 thread_local 混装 4 static 分属三个子系统，
  盲切丢段——条目索引 + git 原文逐字恢复；多轮正则补丁会弄乱文件，
  整文件重写优于补丁。
- **P2 ✅ 已完成**（55571cb）：Host → host.rs（1176 行叶模块），
  exec.rs 1673→557（纯顶层驱动）。
- **P4 ❌ 评估后放弃**：classic linker 三实例（exec.rs wasmi 顶层 /
  exec_wasmtime.rs wasmtime / classic/wasmi.rs proc 池）——逻辑已
  100% 单点化（Host 方法），剩余为注册胶水（每闭包 1-5 行直调）。
  trait 化成本 > 收益（P3-lite 同标准），接受为「类型使然的胶水」。
- **P5 jitsh/rtcl 分工（文档化即可）**：jitsh = 差分 oracle（judge
  三向的第二引擎执行体，hook 全量安装）+ fast-tier 专用 bench 入口；
  rtcl --features jit = 生产 CLI（钩子可选挂载）。二者共享同一
  执行机器（本 crate），不物理合并。

## 合并终态（2026-11-01）

| 层 | 文件 | 行数 | 内容 |
|---|---|---|---|
| 共享逻辑 | fast/shared.rs | 314 | FastShared + sh_*（fast 语义单一真值） |
| 共享宿主 | host.rs | 1176 | Host（classic/fast 两线共用的 ABI v2 方法全集） |
| fast 驱动 | fast/wasmi.rs + fast/wt.rs | 678+680 | 引擎胶水 |
| classic 池 | classic/{mod,wasmi,wt}.rs | 51+425+210 | 模块缓存 + 两引擎 proc 池 |
| 顶层驱动 | exec.rs | 557 | run_top/execute/finish |
| 钩子 | proc_exec.rs | 101 | install + 策略 + 引擎分派 |
| 契约 | abi.rs | 247 | 单一真值（= towasm 产物契约） |

（原 proc_exec.rs+exec.rs = 3329 行混装 → 职责单一化后 4220 总行
含 host.rs 1176；净变化 +~50，换来的是每个子系统的边界可读。）

## 4. 风险登记

- P3 的 `Caller` 类型是 trait 关联类型后，func_wrap 闭包的推断压力
  大——先小样（binary_op 一个闭包）验证 trait 形状再铺开。
- wasmi `Caller::get_export(name)` 单参 vs wasmtime 两参——trait 收敛。
- fast-tier 的 `runs: Vec<TypedFunc>` 联合生命周期——P3 用
  `ProcMainCall` 抽象后 vec 装引擎句柄。
- 每期必须留 `git checkout --` 止损点；P3 若 45 分钟无实质进展，
  记 blocker 跳 P4（P4 独立可做）。

## 5. 与 towasm（Lane B）的接口

towasm 只消费：`emit::compile`（管线）+ `exec::execute_artifact_wasmi`
/`exec_wasmtime::execute`（回环验证）。合并不得破坏这两个 pub 面；
ABI（abi.rs）是产物契约真值——P1-P4 全程不动 abi.rs 语义。
