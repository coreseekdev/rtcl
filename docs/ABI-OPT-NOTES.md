# rtcl ABI 优化备忘录（JIT/WASM 面）

> **状态**：定稿（2026-11-19）。本文是「思考优化 ABI」的交付：现状盘点
> + 已做优化序 + 候选清单（含 feat/rtcl-wasi 蓝本借鉴，逐条
> 内容/收益/成本/风险/时点）。候选 a（ABI 版本 export）本轮落地，
> 其余候选定性归档。
> 关联：`docs/JIT-WASM-GC.md` §3（ABI v2 定稿）、`HANDOFF.md`
> §10duodevicesimus（fast 调用协议）/§10vicesimusprimus（C-float）/
> §10vicesimustertius（wasi 蓝本激活）、`crates/rtcl-jit/src/abi.rs`
> （单一事实源）。

## 1. 现状：M1' ABI v2 面

宿主/JIT 模块契约（`docs/JIT-WASM-GC.md` §3，实现在
`crates/rtcl-jit/src/abi.rs`）：

```wat
(global (import "rtcl" "fuel")    (mut i64))   ;; 步数预算: 回边自减, 宿主充值
(global (import "rtcl" "epoch")   (mut i64))   ;; 失效守卫: 入口+回边比对
(global (import "rtcl" "caching") (mut i32))   ;; 变量缓存许可: 首次 rtcl.call 清零
(import "rtcl" "unbox_int"   (func (param i32) (result i32 i64)))
(import "rtcl" "unbox_float" (func (param i32) (result i32 f64)))
(import "rtcl" "box"         (func (param i64)  (result i32)))
(import "rtcl" "box_float"   (func (param f64)  (result i32)))
(import "rtcl" "binary_op"   (func (param i32 i64 i64) (result i64)))
(import "rtcl" "get_var"     (func (param i32)  (result i64)))
(import "rtcl" "set_var"     (func (param i32 i64)))
(import "rtcl" "call"        (func (param i32 i32 i32) (result i64)))
(import "rtcl" "set_result"  (func (param i64)))
```

- 值表示 = tagged i64（bit0=1 → i63 立即数；bit0=0 → arena 句柄；
  **bits==2 = 错误哨兵**——宿主置 pending error，JIT `br` 到错误出口）。
- `run() -> i32` 返回 Tcl result code（0-6；`STALE = 5` 为 epoch 失效专用）。
- M1' 加法性扩展（冷路径/驱动协作，§3 签名原样保留）：refuel/fail/
  bool_test/incr_var/push_const/exit + M2 时代 concat/foreach 四件套/
  eval_script/eval_expr/lappend_var/call_argv/resolve_proc/
  proc_exit_frame/sync_var——共 27 import（`abi::import::COUNT`）。
- 消费面四层：`abi.rs`（常量/签名索引单一事实源）→ `pass/assemble.rs`
  （发射）→ `exec.rs`（wasmi 宿主）与 `exec_wasmtime.rs`（wasmtime
  宿主，`register_wasmtime_abi` 共享注册体）→ `fast/`（fast-tier 共享
  Store 驱动）。rtcl-wasm 另有 JS 直通面 `src/jit_abi.rs`。

## 2. 已做优化（序）

| 优化 | 内容 | 实测 | 记录 |
|---|---|---|---|
| 立即数零往返 | tagged word 同构，PushInt 免 `push_int` 调用（M0 模式废弃） | — | §3/§4 |
| wasm→wasm 直通 | 宿主裸 `extern "C"` 导出直塞 import object，免 JS trampoline 与 i64↔BigInt 往返 | 直通 1.46 ns/call vs trampoline 11.41（V8） | §3、jit_abi.rs |
| fast 调用协议五件 | env 标志缓存 / 宿主算术 expr_ops 直呼（免文本重建重解析）/ `jit_frame_param_values` 帧槽直读 / fast_try 槽命中快臂 / Div-Mod 双 imm 快臂 | oo_bench2 3.40×→**1.26×**、oovars 4.11×→**1.89×** tclsh | §10duodevicesimus |
| 守卫面硬化 | slow_br 深度参数化（Add/Sub 溢出、Mul 大操作数、Div/Mod 除零、移位域统一）；Div `0÷0` 落宿主 domain error、`7.0/0` IEEE Inf 内联、比较语境 exact_only | 392 例符号矩阵三方逐字节 | §10vicesimusprimus 修复5-8 |
| M2 slot 单态缓存 | classic 值栈 icache（LoadLocal 守卫一次成功 → wasm local 缓存） | var_incr/arith_loop 赢面 | §10senioquincesimus |
| M3 epoch 遮蔽感知 | 折叠在册改名/删除后的站点（Tier1 bump 信号接 epoch gate） | e1/e2 探针口径 | §10undequincesimus |

## 3. 候选优化（含 wasi 蓝本借鉴）

### a. ABI 版本 export（wasi 版本协商模式移植）——**本轮落地**

- **内容**：发射的 JIT 模块新增 export `rtcl_abi_version() -> i32`
  （常量 = `(major << 16) | minor`，低 16 位 minor / 高 16 位 major；
  当前 v2.0——M0 三函数时代记 v1）。宿主（wasmi `exec::execute`、
  wasmtime `register_wasmtime_abi` 共享注册体）实例化后**立即**调用
  比对：major 不等 / 模块 minor 超前 → 明确报错拒绝执行。rtcl-wasm
  dispatcher 模块（pkg）export 同款 u32 函数，`www/rtcl-worker.js`
  加载后比对，不匹配 → init-error（防 HTTP 缓存旧模块 + 新 worker
  的错配——`?v=` 参数只是缓解，这是硬校验）。
- **蓝本来源**：wasi 线 C ABI 面 `rtcl_abi_version`（abi-v2 feature：
  `ABI: 2` 协商过，init(2)→handle 1，HANDOFF §10vicesimustertius）。
  本家无版本字段的根因：宿主与模块同构建发布（in-process 发射），
  从未需要；towasm 产物（跨引擎/跨构建消费）与 pkg/worker（HTTP
  缓存）两个面把「同构建」假设打破了。
- **收益估计**：防错配（正确性护栏，非性能）；产物消费方（towasm
  roundtrip、第三方引擎、缓存 skewed 的浏览器）拿到不合 ABI 的模块
  时从「未定义行为/费解 trap」变为一行明确报错。
- **成本**：~10 行核心（常量 + 比对函数 + 发射 3 段）+ 双引擎接线 +
  测试；零运行时热路径成本（每次实例化一次调用）。
- **风险**：低。旧产物（无此 export）→ get_typed_func 失败 = 同样
  明确拒绝（预期行为，非回归）。快车道路径不做检查——in-process
  发射不可能错配，检查无意义（记注释）。
- **时点**：本轮（perf/wave2）。实现见 §4。

### b. callsite_in_loop 回落协议成本（oo2 1.26× 剩余主因）

- **内容**：v5 gate 按实测拒绝「循环内命令分派」单元走 wasm
  （dict_ops 23×/list_ops 9× 亏，M2 前既有数据——每迭代 var 缓存
  失效 + 分派往返远慢于解释器原生分派），整单元回落解释器。代价：
  oo 方法调用每笔全额重入解释器分派（call_proc 帧机器 + 分派表），
  fast 协议只能在此之上做加法。候选 = 循环内 callsite 的**轻量重入
  面**：直连分派（resolve_proc 槽命中）形态的窄通道，命令体仍宿主，
  但免 eval 原文全链（站点原文→parse→compile 每调用一轮）。
- **收益估计**：oo2 余量 ~126 ns/调用（协议 60 + wasmi 过桥 45 +
  体宿主 Add×2 30）；轻量重入若砍掉分派重入的一半以上，1.26× →
  逼近 1.0×。oovars 同向受益。
- **成本**：大——新的宿主/模块往返协议 + 分派旁路语义面（trace、
  epoch bump、错误帧归属、caching 失效纪律全部要在窄通道里复刻）。
- **风险**：高。分派语义是解释器最深的面；v5 的实测亏数据说明
  粗粒度方案已被证伪，只能走窄通道，而窄通道的语义复刻清单正是
  bug 温床（fast 协议五件每一件都有差分门双杀史）。
- **时点**：reopen 级——归「oo 分派自身 wasm 化 / M5 形态」线，
  不在本协议面再挖（§10duodevicesimus ≤1.0× 定调原文）。

### c. 链接槽出口 flush（oovars 1.89× 剩余）

- **内容**：oovars 的出口 flush set_var（object `variable acc` 经 OO
  prelink 成链接槽）是链接槽落盘的承重件——fast Store 无 clearing
  写点，值只写 wasm 缓存区，靠出口 flush 回写 interp 帧；upvar 被调
  方写必须先落盘，调用方 resync 才读得到（uv2 纪律，
  `abi::cache::SLOT_MASK_OFF` 脏位掩码为此设）。
- **收益估计**：oovars 出口 flush 是其 1.89× 里可观的剩余项（精确
  占比未单独画像——止损评估时未拆）。
- **成本**：中。候选增量方案 = 链接槽脏位直写 wasm 缓存态（把
  per-depth SLOT_MASK 语义推广到链接槽），flush 只对脏位结算。
- **风险**：高。**语义承重件**——动它 = 动 upvar/OO 别名可见性时序；
  §10duodevicesimus 已评估后明确不做（「改值形态=语义变化」同批）。
- **时点**：reopen 线索挂账，须先有 upvar×OO×JIT 的差分专项护栏
  （现 judge 覆盖不针对 flush 时序）再议。

### d. alloc/free 显式面（wasi 借鉴项）——**定调不采纳**

- **内容**：wasi 线 C ABI 面含 `rtcl_alloc`/`rtcl_free`——宿主侧
  显式管理 guest 线性内存的生命周期。本家对应面 = handle-arena +
  tagged word 值域：arena 段（UNSET/哨兵/EMPTY/常量池预驻留）+
  `run()` 返回后 truncate 到水位线（§3 纪律）；堆对象 ValueInner
  手写 RC + copy-on-write + 线程本地 freelist
  （`rtcl-vm/src/value.rs`），宿主 RC 即生命周期，模块内无分配器。
- **为何 wasi 需要而本家不需要**：wasi 形态是「树走查解释器整体
  编译进 wasm」——全部值活在 guest 线性内存里，宿主无法直接管理
  其生命周期，只能要显式 alloc/free。本家模块只持快车道（热循环
  体），值所有权跨边界即移交、arena 水位线批量回收，宿主 freelist
  承接对象级复用。
- **收益估计**：采纳 = 零（负收益：把宿主 RC 的即时析构语义
  （变量 trace、命令删除回调依赖）换成手动管理，正确性风险换不来
  尺寸/速度）。
- **成本/风险**：不适用（不采纳）。
- **时点**：定调归档。引用 `docs/JIT-WASM-GC.md` §1（RC 天然可行、
  WasmGC/tracing 的陷阱清单）与 §2（tagged word 同构 + 边界移交
  两原则）——本家 arena 模型在设计与实测（六基准 ≤1.0× tclsh）
  两面均优。

## 4. 本轮落地记录：候选 a（ABI 版本 export）

- `crates/rtcl-jit/src/abi.rs`：`ABI_VERSION`（v2.0 = 0x0002_0000）
  + `abi_version_compatible(module) -> Result<(), String>`（major
  必须相等；模块 minor 超前 → 拒绝（宿主不认识新面）；模块 minor
  更小 → 放行（minor = 加法性扩展，宿主向后兼容））。编码约定
  （低 16 minor / 高 16 major）写常量注释。
- `crates/rtcl-jit/src/pass/assemble.rs`：模块尾追加版本函数
  （复用 `ty::RUN` 签名 `() -> i32`；函数索引 = 局部段末——fast
  形态 30 / 常规 28，不触碰 `import::RUN/FAST_IMPL` 索引）+ export
  `rtcl_abi_version`。
- `crates/rtcl-jit/src/exec.rs`（wasmi）与 `exec_wasmtime.rs`
  （wasmtime，`register_wasmtime_abi` 内——顶层与 proc 池同享）：
  实例化后立即 `get_typed_func("rtcl_abi_version")` → 调用 →
  `abi_version_compatible`，Err → panic 明确报错（in-process 发射
  恒匹配，此检查的真实靶 = towasm 产物与跨构建消费方）。fast
  驱动不做检查（in-process 同理，记注释）。
- `crates/rtcl-wasm/src/lib.rs`：`#[wasm_bindgen] pub fn
  rtcl_abi_version() -> u32`（pkg 面同款 v2.0；与 JIT 发射面同一
  编码约定、同一常量值——pkg 的 wasm-bindgen 面与 JIT 发射面同仓
  同发布，单一版本空间避免双账本）。
- `crates/rtcl-wasm/www/rtcl-worker.js`：import 解构加
  `rtcl_abi_version`，`await init()` 后比对——非函数（旧模块）或
  不等（缓存 skew）→ throw → 既有 catch 链 → `init-error` 明确
  报错（报错文案带 `?v=` 硬刷新指引）。worker 面取**严格相等**
  （它守的错配形态 = HTTP 缓存 skew，不存在合法的版本差）。
- 测试：`crates/rtcl-jit/tests/abi_version.rs`（发射产物 export
  存在性 + 比对函数矩阵 + run_top 端到端）；worker 面随真机回归
  （init-error 路径 node 侧轻验证）。
