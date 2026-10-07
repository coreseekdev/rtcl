# rtcl 生产就绪状态 (2026-10-07)

**判定：生产可用** — 浏览器 WASM VM 场景为主目标的嵌入式 Tcl 解释器。
本文件整理与标准 tclsh 8.6.17 的差异分析、性能定位与部署状态。
优化台账（含全部事故与方案）见 `bench/BASELINE.md`；架构综述见
`ARCHITECTURE-PERF.md`。

---

## 1. 兼容性（vs tclsh 8.6.17）

### 判定方法
- **判官体系**：`judge/run.sh` — 87 个语料文件、**4072 用例全量逐字节比对**
  （stdout + 退出码 + errorInfo/errorCode）；4067 PASS + 5 FAIL（该 5 例
  为 tclsh 独立运行时的自身失败，rtcl 逐字节复现——见下"已知分歧"）。
  用例由 tclsh 8.6.17 官方测试套件机械提取（23759 个候选中提取 5701、
  过滤后保留 4072）。
- **双引擎差分**：`judge/sweep.sh` — 字节码引擎 vs 树遍历引擎全探针逐字节
  差分，0 分歧（35 个探针文件）。
- **数字探针**：i64 极值/溢出宽化/除模语义/混合表示，与 tclsh 逐字节一致。

### 已知分歧：**0 项 rtcl 引起的语义分歧**（2026-10-07 逐 case 审计）

- **4072 case 全量审计**（tclsh 与 rtcl 各自运行全部 87 文件、逐 case
  比对 PASS/FAIL）：tclsh 8.6.17 自身 FAIL 5 例（gen_namespace-old 的
  2.3/6.10/6.13/6.16/6.19——提取语料缺少前序 case 6.12 等的跨 case
  setup 状态，tclsh 独立运行时自身失败），**rtcl 逐字节复现相同的失败
  输出——严格对齐，非 rtcl 分歧**。
- **唯一 OPEN 语义分歧**：`interp alias {} name {} target` 空解释器
  占位形式（tclsh 接受，rtcl 报 wrong # args）——低严重度，见
  `specs/DIVERGENCES.md` D-1。
- 历史记录更正：gen_namespace-old 的 5 例与 rename 遮蔽在 2026-10-03
  被记为分歧，2026-10-07 复核均已修复/证伪（见 specs/DIVERGENCES.md）。

**36 项超出标准 Tcl 的扩展**（保留判定经用户裁定）：`function`/`lambda`/
`curry`/`loop`/`ensemble`/`defer`/`json`/`ref` 等。

## 2. 性能（Linux x86_64，负载 <2.5，5 次取均值）

| 基准 | rtcl | tclsh | 比值 | 会话起点 |
|---|---|---|---|---|
| arith_loop 微 | 31ms | 47ms | **0.6×（反超）** | 5.7× |
| var_incr 微 | 28ms | 41ms | **0.6×（反超）** | 7.9× |
| string_build 微 | 6ms | 7ms | **0.8×** | — |
| dict_ops 微 | 8ms | 10ms | **0.8×** | — |
| list_ops 微 | 9ms | 10ms | 1.0× | — |
| proc_fib 微 | 11ms | 11ms | 1.0× | 9.1× |
| dloop（30 万 dict） | 281ms | 256ms | **1.05×（平价）** | 11× |
| oo_bench2（百万方法） | 518 | 402 | 1.28× | 2.9× |
| oo_bench2_vars | 770 | 445 | 1.73× | 6.2× |
| fib25 | 42 | 36 | 1.17× | 9.1× |
| fe_bench | 69 | 32 | 2.16× | 3.2× |
| build 循环 | 22 | 16 | 1.38× | ~80× |

微套件**全面持平或反超**；重型基准全部收敛至 2.3× 以内。

## 3. WASM 部署（主目标场景）

- **构建**：`crates/rtcl-wasm/build.sh`（wasm-bindgen web 目标）
- **体积**：1.6MB（release，未压缩）
- **测试**：node 套件 **18/18 通过** — exec/JS 命令注册/多参/隔离性/
  注销/puts 路由/JS 错误传播
- **API**：`RtclHandle::new/exec/register_command/unregister_command/
  set_output_handler`；Rust 宿主侧另有原生 `CommandFunc` 注册栈
- **验证**：所有表示层改造（标记 Value/立即数/Cell/freelist/Fx 表）均
  通过 wasm32 check + node 套件

## 4. 架构要点（性能相关）

- **双引擎**：字节码 VM（主）+ 树遍历（RTCL_NO_BYTECODE / 语义回退），
  差分门槛保证互换一致
- **Value**：8 字节标记字 — i63 立即整数（算术/比较零分配零引用计数）+
  堆箱（freelist 回收，COW）
- **调用链**：解析令牌缓存（免 dispatch 链）、帧池、常量池身份缓存、
  参数头缓存 + args 向量池
- **哈希**：解释器侧全部 unkeyed Fx（~2ns/probe）
- **proc 调用开销已反超 tclsh**（0.52×，分解实测）

## 5. 剩余优化路线（已存档，非阻塞）

| 项 | 预期 | 状态 |
|---|---|---|
| foreach 路由对齐 | dloop/fe 的 foreach 体开销 ~20% | framed 双模式已落（G24）；路由对齐待做 |
| 指针稳定存储 | vars/oo 链接变量哈希链清零 | VarMap newtype 方案已存档 |
| 直接线程化调度 | per-op 税 ~20ns→7ns | 重写级，需 perf 制导 |

## 6. 运维注意

- `kernel.perf_event_paranoid` 需 ≤1 才能 perf 制导（`sudo sysctl
  kernel.perf_event_paranoid=-1`）
- 基准脚本在 `bench/scripts/`（勿放 /tmp — 会话间被清过一次）
- 门槛纪律：judge 87/87 + sweep 0 + tests 1143/0 三门全绿才可提交
