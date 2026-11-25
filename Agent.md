# rtcl Agent 工作指南（AI 协作者须知）

本文档供 AI 助手在 rtcl 项目工作时的操作规程。**先读完再动手**——
其中每条规则都来自真实事故。

## 项目概况

rtcl 是浏览器可嵌入的 Tcl 解释器（Rust），主目标为 **WASM VM**，
对标 **Tcl 9.1**（oracle：tclsh 9.1.0，官方 release
[core-9-1-0](https://github.com/tcltk/tcl/releases/tag/core-9-1-0)）。
judge/probes/extract 的 oracle 已默认指向仓内构建的
`.refer/tcl910/inst/bin/tclsh9.1`；`JUDGE_ORACLE=tclsh` 可指回系统
8.6.17 跑历史基线。**当前状态**：judge 88/88 语料文件对 9.1.0 逐字节
一致；探针 312/437（125 项 8.6→9.1 语义差 = 迁移面，见 TODO.md
「对标迁移」）。生产可用基线（对 8.6.17 建立）见 `docs/PRODUCTION.md`。

### 关键事实（动手前必读）
- **判官**：`judge/run.sh` — 88 语料文件与 oracle（默认 tclsh9.1 =
  9.1.0；`JUDGE_ORACLE` 可覆盖）**逐字节**比对（stdout + 退出码 +
  errorInfo/errorCode）。语料由 8.6.17 官方测试套件机械提取
  （`judge/extract/`，case 级 4067/4072 基线是对 8.6.17 建立的），
  期望运行期求值，对任意 oracle 可运行。
- **双引擎**：字节码 VM（`vm_exec.rs`，主）+ 树遍历（`loops.rs`/
  `eval.rs` 等，回退 + `RTCL_NO_BYTECODE`）。两引擎必须逐字节一致 —
  `judge/sweep.sh` 是差分门槛（0 分歧）。
- **Value 表示**：8 字节标记字（`rtcl-vm/src/value.rs`）— bit0=1 为
  i63 立即整数（算术/比较零分配），bit0=0 为堆箱（freelist 回收 +
  COW）；`bits==0` 是槽哨兵（UNSET）。**Drop/Clone/访问器对三种状态
  的分派必须完备**——漏一类就是空指针（判官当场抓过）。

## 工作检出行

**用 /home/nzinfo/src.note/rtcl-master**（不是 ~/src.note/rtcl——
那是旧检出）。**harness 的 shell cwd 每条命令重置回 ~/src.note/rtcl**，
所有路径用绝对路径或每条命令先 cd。

- **rtcl-master 是多代理共享检出行**：未提交的工作树改动会在命令间被
  其他代理的 checkout/reset 抹掉（发生过，执行器臂整段消失）。**每个
  绿色里程碑立即 `git add -A && git commit`**；别把源码改动裸留在
  工作树里跨命令。
- 基准/探针脚本放 `bench/scripts/` 或 `judge/probes/`（提交）——
  放 /tmp 会被会话清理（发生过，整个基准集丢失、测量全部失效）。

## 构建与门槛纪律

```bash
# 构建（内存上限必须带——机器会 OOM）
bash -c 'ulimit -v 3400000; cargo build --release -j 2'
# 测试（1481 个，63 套件）
bash -c 'ulimit -v 6000000; cargo test --workspace -j 2'
```

- **门禁脚本已入仓**（原 target/audit/ 下的散件，worktree 相对路径，
  任何 worktree 直接用；日志写 `<worktree>/target/audit/`）：
  `bash judge/gates/quick-gates.sh <tag>`（judge、crash-gate、probes、
  rtcl-vm lib 两种特性、五种特性组合 check、wasm32 构建，约 5 分钟，
  日志 quick-<tag>.log）；`bash judge/gates/all-gates.sh <tag>`（上面
  加 sweep 与 workspace 测试，约 25 分钟，放后台，末行 `ALLDONE`）；
  `bash judge/gates/rest-gates.sh <tag>`（只 sweep + workspace 测试）；
  `bash judge/gates/bench/run.sh <tcl>`（tclsh 与 rtcl 两引擎并排
  µs/迭代 + 峰值 RSS；`BENCH_TCLSH` 可换 oracle）。
- **多个会话共用仓库：每个 agent 一棵独立 worktree**
  （`git worktree add ../rtcl-<名字> <分支>`），做完再合。提交前
  `git status --short`，不属于你的改动不要暂存；共享文件只挑自己的
  hunk（`git diff FILE > p` → 删别人的 → `git apply --cached p`）。
- **worktree 硬链接克隆 target 的假新鲜陷阱**（2026-11-26 实证）：
  `cp -al target` 克隆后多树并行构建，fingerprint 状态会被另一棵树
  的构建经共享 inode 就地改写——cargo 报 `Finished 0.06s` 全 fresh
  但 `target/release/rtcl` 时间戳还是旧的。**每次构建后核对二进制
  时间戳**；没变就 `find crates -name lib.rs -path "*/src/*" -exec
  touch {} + && touch crates/rtcl-cli/src/main.rs` 强制重建。新
  worktree 首次构建前也可直接 `rm -rf target/release/.fingerprint`
  一劳永逸（代价是多编译几秒）。
- **判官/差分与构建绝不同时跑**（OOM）。
- **陈旧二进制假绿**：`grep -cE "^error"` 有匹配时退出码为 0，`&&` 链
  会带陈旧二进制继续过门（一个会话里中过三次）。构建后
  `ls -la target/release/rtcl` 看时间戳再信门。
- **提交门槛**（2026-11-25 起，oracle = tclsh 9.1.0）：judge 88/88 +
  crash-gate 全过 + sweep 不新增 DIFF（既有 12 处名单见 HANDOFF）+
  tests 1481/0 + wasm32 构建无错误 + 动 wasm 面时 crash-wasm 52 与
  node test 26 全过；probes 对 9.1 的 125 项 DIFF 是迁移清障账
  （TODO.md「对标迁移」），不新增即可。提交信息
  `type(scope): 中文摘要`，正文写改法与没动的地方、`终态：` 一行列
  门禁数字，带 `Co-Authored-By: ZCode (GLM-5.4) <noreply@localhost>`。
- **崩溃门禁**：`bash judge/crash-gate.sh`——`judge/crash-probes/` 里每个
  探针钉住一个脚本可触发的崩溃/挂起/静默错误（记录见
  `specs/CODE-AUDIT-20261123.md`），与 judge/sweep 同跑。再发现这类缺陷
  时先加探针（基线必须先红）再修。
- **wasm 侧同类门禁**：`cd crates/rtcl-wasm && ./build.sh && cd ../..
  && node judge/crash-wasm.mjs`——每项用全新实例（trap 之后实例不可
  用）。动到递归/嵌套/限制、`rtcl-wasm/src`、或 `.cargo/config.toml`
  的 wasm 段时必跑。
- **wasm 的递归上限**：约束是 JS 引擎的调用栈（约 1MiB），不是线性
  内存——`-zstack-size` 不解决问题。wasm32 默认上限 150 是实测校准
  的（最坏形态 `apply` 套 `catch` 在 200 层溢出），别改回 1000。
- **sweep 的 `ulimit -v` 坑**：`ulimit -v 6000000` 下 wasmtime 的
  `stub memory: mmap failed to reserve` 会让 rtcl-towasm 的 5 个测试假红
  ——测试（非构建）阶段去掉上限，并用 `--no-fail-fast` 跑全。
- **调试探针（eprintln/env 检查）绝不留在每 op 路径上**——一次 env 查找
  ×每 op = 8× 回退，且门禁不抓（相对回归），只有直接重测才暴露。

## 性能工作流

1. **先读 `bench/BASELINE.md` 尾部**——当前板、已定界的剩余杠杆、
   负结果清单（防重复推导）全在那里。
2. perf 需 `sudo sysctl kernel.perf_event_paranoid=-1`（无密码 sudo；
   被锁时只能墙钟分解）。符号化构建：
   `CARGO_PROFILE_RELEASE_STRIP=false RUSTFLAGS="-C force-frame-pointers=yes -C debuginfo=1"`，
   产物建到 /tmp/g8sym 类目录（master 的 target 是 stripped）。
3. 负载 <2.5 才做基准（外部 node/gcc 任务会把 tclsh 参照推高 1.4×）；
   同窗交错测量。
4. **基準脚本放 `bench/scripts/` 并提交**。
5. 表示层/语义改造的模式：先写探针钉死 tclsh 行为 → 实现 → 五门全绿
   （judge/sweep/tests/探针/feature matrix）→ 即时提交。历史上判官
   三次抓到进程内测试全绿但表示层有双 free/空指针的版本。

## 语义改动的特殊纪律

- **错误信息/errorInfo 是字节级语义**：帧文本、行号、装饰顺序全部由
  语料钉死。改动 foreach/循环/异常路径前，先读
  `foreach_inline_tests`（vm_exec.rs）与 loops.rs 的 tclsh 探针注释。
- **双引擎一致性**：任何执行器语义改动，sweep（差分）必须 0。树的
  foreach_lexical/dispatched 路由是上下文条件的（probe 钉死）——
  别凭读码推断，跑 sweep。
- **framed 双模式**（G24）：全局编译的 foreach 用带帧错误协议（body
  退出帧 + setting 装饰），proc 内用无帧协议。`ForeachInfo.framed` +
  `Region::BodyFramed` 是这个机制的载体。

## 已知边界（不要"顺手修"）

- rtcl-core 的 `embedded` no_std 构建破损（G5 起记录在案，非本阶段）。
- 5/4072 语料用例与 tclsh 有已记录分歧（见 docs/PRODUCTION.md）。
- JIT（rtcl-jit）暂停——解释器为主要引擎；别在 perf 会话里动它。

## 提交与推送

- `git push origin master`（工作在 master 直推模式）。
- 大改前 `git worktree add` 到 /tmp 分支也行，但 **master 直推 +
  每步即时提交**是本仓的既定模式。
- 性能台账 `bench/BASELINE.md` 追加式记录（每轮 G# 一节）；
  `TODO.md` 与之同步。
