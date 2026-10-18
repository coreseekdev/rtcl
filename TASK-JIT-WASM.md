# 任务包：rtcl WASM Compiler / JIT（M1' 起）

> **用途**：把「Tcl → WASM 编译/JIT」任务完整委托给一个新 agent。
> 本文件自足：读完本文件 + 指定资料即可开工，无需对话历史。
> **状态**：设计已定稿（`docs/JIT-WASM-GC.md`）。**M1' 已完成、M2 已落半**
> （原语/foreach/lmap 内联 8e1034d + 真实接受路径修复 f25d234；judge 三向
> 87/87 双引擎）。解释器性能两批（PR #1/#2，2026-10-18/19 合并）已榨干，
> 施工恢复。**剩余 = M2 后半（proc body + slot 单态 int 缓存）与 M3（epoch
> 接 Tier1 bump）**——两者的协调者侧接口改动在 `jit/proc-body-epoch` 分支
> 处理。创建：2026-10-17；本段更新：2026-10-19。
>
> **地壳变动（2026-10-18 起，重启前必读）**：
> - crate 布局：解释器已整体迁入 **rtcl-vm**（rtcl-vm 即 VM），rtcl-core
>   退为 facade——本文件旧锚点中的 `rtcl-core/src/interp/**` 一律读作
>   `rtcl-vm/src/interp/**`；演示 VM `rtcl-vm/src/execute.rs` 已删除。
> - 解释器新增融合超指令族（G/M/U 轮）；JIT 编译边界统一经
>   `ByteCode::unfused()` 取规范窗口——**接受集零改动**，契约与逐 op 表
>   见 `docs/JIT-OPCODE-DELTA.md`（含 Round U 增补）。
> - judge 语料现为 **88 文件**（新增 `judge/corpus/oo_instance_vars.tcl`）。

## 0. 一分钟定位

- rtcl = Rust 实现的 Tcl 解释器（jimtcl 血统），主 target =
  **wasm32-unknown-unknown**，另支持 native / wasip1 / embedded no_std。
- 本任务 = 建设 `rtcl-jit` crate：**ByteCode → wasm 模块**，per-proc 粒度，
  运行时生成并实例化（Mono jiterpreter 模式）。
- **M0 已落地**（2026-10-03，commit `0b5177a`）：crate 骨架、wasm-encoder
  发射最小模块、wasmi round-trip 测试 5/5 绿。**勿重做，在其上演进。**
- 设计宪法 = **`docs/JIT-WASM-GC.md`**。本文件只做任务定义与纪律；
  一切技术细节以该文档为准（它包含 ABI v2 全文、快车道 lowering 表、
  nanopass 管线、业界证据链）。

## 1. 必读清单（按序）

1. `HANDOFF.md` §0/§1/§5/§6 —— 项目状态、基础设施、阶段一裁决、JIT 裁决
   全文（含 2026-10-17 设计定稿摘要）。
2. `docs/JIT-WASM-GC.md` —— 重点 §1（为什么不用 WasmGC）、§2（三原则 +
   「回退」两粒度术语约定）、§3（ABI v2）、§4（Int 快车道）、§7.3
   （nanopass 管线拆分）；§5/§6 是后续里程碑，开工前浏览即可。
3. `Agent.md` —— 仓库级纪律与提交门槛。
4. 源码锚点（读这些建立手感，勿发散）：
   - `crates/rtcl-jit/src/{lib.rs,emit.rs,engine.rs}` —— M0 现状（~250 行）
   - `crates/rtcl-ir/src/{opcode.rs,bytecode.rs}` —— JIT 的输入 IR
   - `crates/rtcl-vm/src/value.rs:317-524` —— Value = tagged u64 /
     ValueInner / RC / freelist（ABI 设计的物理基础）
   - `crates/rtcl-vm/src/interp/vm_exec.rs` —— ByteCode 的第一消费者，
     逐 op 语义参照物（改语义必须与之逐字节对齐）
   - `crates/rtcl-parser/src/compiler.rs` —— ByteCode 产出端
5. 语义门禁语料（本任务的「考卷」）：
   - `judge/corpus/` —— 10 个手写文件 + 77 个 **Tcl 官方测试集提取文件**
     （`gen_*.tcl`，4072 case；由 `judge/extract/extract.py` 从官方
     tcltest 套件保守提取、oracle 自验证）；
   - `.refer/tcl` —— Tcl 官方源码（行为 oracle 的测试语料来源；若被删
     按 HANDOFF §0 重新 clone）；
   - `specs/*.allium` + `specs/probe.sh` —— 行为规格与逐行对拍探针
     （stdin 喂 Tcl 片段，自动 tclsh vs rtcl diff，改语义时先用它取证）。
6. 已批准的工具链：wasm-encoder 0.261（发射）、js-sys（jit-wasm 引擎）、
   wasmi 2.0（测试引擎）、wasmtime（jit-native，M1' 才接）。

## 2. 里程碑序列与门禁

**M1'（首个执行目标）= ABI v2 整体切换 + Int 快车道 + fuel/epoch
globals + emit.rs nanopass 化拆分**。细则：

- 范围：
  1. ABI v0（三函数 handle ABI）→ ABI v2（设计文档 §3 全文）：值穿越 =
     tagged i64；multi-value 错误标志；错误哨兵 bits==2；`run()` 返回
     Tcl result code（STALE=5 新增）。
  2. Int 快车道（§4 表）：比较免 untag；add/sub 内联 + ~5 指令范围检查；
     mul 高 32 位检查；**div/mod 全走宿主**；位运算内联；守卫失败调
     `binary_op` 宿主兜底（shimmer/bignum/报错全在宿主）。
  3. fuel/epoch imported mutable globals（§3）：循环回边 fuel 自减、
     入口+回边 epoch 比对——**补齐 TODO.md 的 JIT 步数预算缺口**，
     验收含 `while {1} {}` 编译后仍可被 `set_max_steps` 停机。
  4. emit.rs 按 §7.3 拆为单职责 pass 链（L0-L4 IR + 7 pass），每 pass
     配 golden 测试；cpcheck 模式：每层 IR 配校验器。
  5. wasm→wasm 直通接线（§3）：宿主裸 `extern "C"` 导出直塞 JIT 模块
     import object；保留 JS trampoline 路径作对照。
  6. judge/run.sh 新增 `--engine jit` 档（三向差分：tclsh / interp / jit）。
- **开工前置（先测后承诺，设计文档 §8 假设 A/C）**：先写 micro-bench
  实测 (a) wasm→wasm import vs JS trampoline 调用开销（三引擎），
  (b) per-proc 同步编译延迟（<1ms 预算）。结果影响接线与同步/异步
  实例化决策；不达标要回来改设计，不许硬上。
- 验收（全绿才算完成）：
  - `cargo test --workspace` 全绿（当前 ~1143，以实际为准）；
  - **Tcl 官方测试集三向全绿**：`./judge/run.sh` 无回退，且
    `./judge/run.sh --engine jit`（M1' 新建的档位）三向差分 0 分歧。
    语料即官方 tcltest 提取版：文件级 **87/87**、case 级 **4072/4072**，
    tclsh vs interp vs jit 逐字节一致——**官方测试集是门禁，不是抽样
    参考，一个 case 都不许放**；
  - `bench/cases/arith_loop.tcl` jit vs interp 提速 ≥3×（M1 原验收线）；
  - `cargo check -p rtcl-wasm --target wasm32-unknown-unknown` 绿；
  - `while {1} {}` 类死循环在 JIT 路径受步数预算约束（可停机、可恢复）。
- **明确不在 M1'**：float 快车道（`jit-float-lane` 默认 off，属 C-float）、
  lazy-string（独立任务包）、js-string builtins（C-str）、跨 proc 内联、
  WasmGC 值表示、热点 tier-up（M4 已重新定位为「proc 定义即编译」，见
  设计文档 §9）。

**后续里程碑**（做完 M1' 并交付后才启动，各自 gate 同 M1'）：
M2 proc body + 原语 + slot 单态 int 缓存（§7.3 alloc_locals）；
M3 epoch 接 Tier1 bump 信号（rename/redefine 风暴 fuzz）；
M4 定义即编译常态化 + 可选画像增强重编译；
C-float / C-str / C-lazy 见设计文档 §5/§6/§9。

## 3. 硬约束（违反即返工）

1. **语义最高裁判 = 系统 tclsh 8.6.17 实测**（`echo '...' | tclsh`），
   不凭记忆。一切输出（含 errorInfo）逐字节对齐。
2. **不引入新依赖**（rtcl 是嵌入式定位）。wasm-encoder/js-sys/wasmi/
   wasmtime 已批，其余先问。
3. **不顺手重构解释器**；不改 ByteCode 语义——如确需改，必须同步
   `rtcl-vm/src/interp/vm_exec.rs`（活路径；原演示 VM `execute.rs` 已
   删除，无第二参照），并在回报中显式声明。
4. **帧共享纪律**：守卫失败只发生在 op 边界；值在宿主 arena、变量在
   宿主 frame；**永不把未物化的 JIT 私有状态带出 guard 点**（设计文档
   §7.9 模式 3）。
5. **「编译仍全量、运行时快慢分流」**（M0 的编译级全有/全无已按设计
   文档 §2 修订）；但单个 op 无法 byte-exact 时，该 op 必须走宿主
   兜底而非发明新语义。
6. **解释器仍是主引擎**：JIT 仅经 feature（jit-wasm/jit-native）接入，
   默认构建行为与性能不得改变；`RTCL_NO_BYTECODE` 类逃生门保持有效。
7. **阶段性提交（本任务包例外，用户裁决 2026-10-17）**：仓库默认纪律
   「子任务不 git commit」对本任务**豁免**——长周期构建任务必须按
   检查点自行 commit，确保任何时刻可回退到最近的绿色状态。
   规则见 §3a。
8. **文件互斥**：分配给你的可写范围 = `crates/rtcl-jit/**`、
   `crates/rtcl-wasm`（仅 ABI 导出与 import object 接线）、
   `judge/run.sh`（加 `--engine jit` 档）、`bench/cases|scripts`
   （新增 case）、`crates/rtcl-jit/tests/**`。其余文件只读；
   确需改动（如 value.rs）时停下来回报，由协调者决定。
9. **不许为过关动语料**：`judge/corpus/**` 与 `judge/extract/extract.py`
   的提取协议只读。发现语料本身问题（如 tclsh 独立运行的上下文失败）
   按 HANDOFF §3 先例定性记录、先回报，由协调者裁决——**考卷不许改，
   只能改答卷**。

## 4. 验收命令速查

```bash
cargo test --workspace                                    # 全绿
cargo check -p rtcl-wasm --target wasm32-unknown-unknown  # wasm32 门槛
cargo build -p rtcl-jit --features jit-wasm               # feature 矩阵
cargo build -p rtcl-jit --features jit-native             # （M1' 起接线）
./judge/run.sh                                            # 无回退
./judge/run.sh --engine jit                               # 三向差分（M1' 新建）
bench/run.sh                                              # bench 不回退
```

oracle：`echo '<script>' | tclsh`（Tcl 8.6.17）。
对拍探针：`echo '<script>' | specs/probe.sh`（stdin 逐行，自动
tclsh vs rtcl diff；改语义前先用它取证）。
快速验证：`./target/release/rtcl -c '<script>'`。

## 5. 返回格式（按 HANDOFF.md §7 纪律）

每项改动回报：设计依据一句话（引用设计文档 §x.y）+ 修改位置 path:line +
验证证据（judge / cargo test / bench 前后数字）+ 未解决项及原因。
阶段性回报 burndown 式数字（多少 op 已入快车道、多少仍走兜底），
不报散文。

## 6. 卡壳升级路径

- 设计文档未覆盖的语义角落 → 先 tclsh 实测取证 → 最小侵入实现并在回报
  中声明；拿不准就停手回报，不发明语义。
- 性能假设（设计文档 §8 A-E）实测不达标 → 停下来回报数字，由协调者
  裁决是否改设计；不许为达标而偷换口径。
- 引擎差异（V8/SM/JSC 行为不一）→ 记录实测矩阵，降级到保守路径，
  回报后再议。

## 7. 派发方式（协调者用）

把本文件路径交给 agent 作为唯一任务输入，附一句：

```
你在 rtcl 项目（/home/nzinfo/src.note/rtcl）执行 WASM JIT 任务。
读 TASK-JIT-WASM.md 并严格按其执行（含必读清单、硬约束、验收门禁）。
首个里程碑 = M1'。开工前先交 §2 的开工前置 micro-bench 数字。
```
