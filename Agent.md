# rtcl Agent 工作指南（AI 协作者须知）

本文档供 AI 助手在 rtcl 项目工作时的操作规程。**先读完再动手**——
其中每条规则都来自真实事故。

## 项目概况

rtcl 是浏览器可嵌入的 Tcl 解释器（Rust），主目标为 **WASM VM**，
同时对齐 tclsh 8.6.17 语义。**当前状态：生产可用**（见
`docs/PRODUCTION.md`）。

### 关键事实（动手前必读）
- **判官**：`judge/run.sh` — 87 语料文件、4067/4072 用例与 tclsh 8.6.17
  **逐字节**比对（stdout + 退出码 + errorInfo/errorCode）。语料由
  tclsh 官方测试套件机械提取（`judge/extract/`）。
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
# 测试（1143 个）
bash -c 'ulimit -v 6000000; cargo test --workspace -j 2'
# 判官 + 差分（在 /tmp/rtcl-master-wt；二进制用 cp 同步——rsync 排除 target）
cp target/release/rtcl /tmp/rtcl-master-wt/target/release/rtcl
rsync -a --delete --exclude target --exclude .git ./ /tmp/rtcl-master-wt/
bash -c 'ulimit -v 8000000; /tmp/rtcl-master-wt/judge/run.sh'
bash -c 'ulimit -v 8000000; /tmp/rtcl-master-wt/judge/sweep.sh'
```

- **判官/差分与构建绝不同时跑**（OOM）。
- **陈旧二进制假绿**：`grep -cE "^error"` 有匹配时退出码为 0，`&&` 链
  会带陈旧二进制继续过门（一个会话里中过三次）。构建后
  `ls -la target/release/rtcl` 看时间戳再信门。
- **提交门槛**：judge 87/87 + sweep 0 + tests 1143/0 + wasm32 check
  四门全绿；提交信息带 `Co-Authored-By: Claude Code <noreply@anthropic.com>`。
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
