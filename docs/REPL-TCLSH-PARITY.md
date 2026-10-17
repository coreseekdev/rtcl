# REPL parity: rtcl vs tclsh 8.6.17

交互行为的逐项对照。oracle 是本机的 tclsh 8.6.17（与 judge/run.sh 的
陪跑 oracle 同版本），rtcl 侧为 `rtcl -i`（rustyline）与 web REPL
（`crates/rtcl-wasm/www/index.html`）。所有 tclsh 输出均来自 pty 实测
（`script -qc tclsh /dev/null`）。

日期：2026-10-17 · rtcl master @ REPL-parity 系列

## 总览

| 行为 | tclsh 8.6.17 | rtcl CLI（现状） | rtcl web | 备注 |
|---|---|---|---|---|
| 主提示符 | `% ` | `% ` | `% ` | 对齐 |
| 多行续读 | 有（未闭合 `{` `"` `[`、反斜杠-换行持续读入） | 有（`rtcl_parser::is_complete` 判定） | 有（`RtclHandle::is_complete`，同一判定） | 对齐 |
| 续行提示符 | **无**（静默继续读） | `... ` | `... ` | **有意分歧**：可见标记便于确认多行状态 |
| 结果回显 | 非空即打，多行值原样多行输出 | 同 | 同 | 对齐 |
| 错误输出 | 仅消息，无前缀，无 errorInfo 栈 | 同（stderr） | 同 | 对齐（栈是 script 模式行为，见下） |
| 顶层 `break`/`continue` | `invoked "break" outside of a loop` | 同（stderr） | 同 | 对齐 |
| 顶层 `return ok` | 回显 `ok` | 同 | 同 | 对齐 |
| 顶层 `exit 7` | 进程退出，rc=7 | rc=7 | 打印 `exit 7`（页面不能退出） | 对齐（web 受限） |
| EOF（Ctrl-D） | 静默退出，rc=0 | 静默，rc=0 | n/a | 对齐 |
| Ctrl-C | 进程被 SIGINT 杀死 | `^C` + 清空当前输入，继续 | n/a | **有意分歧**：REPL 常规体验 |
| `tcl_interactive` | `1` | `1` | `1` | 对齐 |
| `history` 命令 | 有（事件表，可编程） | `.history` 点命令（只读列表） | 输入框上下键 | **已知差距**，见下 |
| `info complete` | 有 | 有 | 有（wasm 直通） | 对齐 |
| 未知命令 | `invalid command name "foo"` | 同 | 同 | 对齐 |

## 逐项实测

### 提示符与多行续读

tclsh 对未闭合的花括号、引号、方括号持续读入，中间**不打印任何提示符**；
补全后整条命令求值，多行结果原样多行回显：

```
% set x {a
b
c}
a
b
c
% puts $x
a
b
c
% puts "unclosed
more"
unclosed
more
```

rtcl CLI 同一输入（pty 冒烟，`... ` 是 rtcl 的续行标记）：

```
% set x {a
... b
... c}
a
b
c
```

完整性判定两端同源：`rtcl_parser::is_complete`（`info complete` 与
CLI REPL 用它；wasm 经 `RtclHandle::is_complete` 暴露给 web）。
边界语义与 tclsh `info complete` 一致：

| 输入 | tclsh `info complete` | rtcl |
|---|---|---|
| `set a 1 \`（裸尾反斜杠） | `1` | 完整 → 立即求值 |
| `set a 1 \<newline>`（尾部） | `0` | 不完整 → 继续读 |
| `set x {` | `0` | 不完整 |
| `set x 1` | `1` | 完整 |

### 结果回显

非空即打；多行值原样多行。`puts` 有输出但返回空串，故只打 puts 的输出。
（见上方转录。）

### 错误输出

**交互模式只打印消息本体**——没有 errorInfo 栈、没有 `(file ...)` 尾：

```
% foo bar
invalid command name "foo"
% error "boom" "boom" {CODE X}
boom
% set e $errorInfo
boom
% puts $errorCode
CODE X
```

`errorInfo` 在顶层错误后只含消息本身（`boom`），`errorCode` 正常传播。
完整栈（`while executing` 帧 + `(procedure "p" line 1)` + `(file ...)` 尾）
是 **script 模式**行为——rtcl 的 `rtcl -f`（`Interp::error_report`）已对齐：

```
$ tclsh err.tcl          /   $ rtcl -f err.tcl
inner                        inner
    while executing              while executing
"error inner"                "error inner"
    (procedure "p" line 1)       (procedure "p" line 1)
    invoked from within          invoked from within
"p"                          "p"
    (file "err.tcl" line 1)      (file "err.tcl" line 1)
```

rtcl REPL 曾打 `Error: <msg>` 前缀并走 annotated-snippet 渲染，现已改
为裸消息（`crates/rtcl-cli/src/repl.rs` + `eval_repl_line`）。

### 顶层控制流

```
% break
invoked "break" outside of a loop
% continue
invoked "continue" outside of a loop
% return ok
ok
% exit 7          # 进程退出，$? = 7
```

rtcl：`break`/`continue` 打同文本（stderr）；`return` 值按结果回显；
`exit N` 映射进程退出码（`repl::exit_code_of`）。script 模式的
`exit 7` rtcl 仍是 rc=1 + `exit with value: 7` —— 这是既有文件模式
语义，**不在 REPL 范围内改动**（judge 语料盯的是文件模式）。

### 退出与中断

- EOF（Ctrl-D）：tclsh 静默退出 rc=0；rtcl 同（不再打印 "Ctrl-D"）。
- Ctrl-C：tclsh 8.6 无 SIGINT 处理，进程直接死；rtcl 打 `^C`、丢弃
  半截输入、继续跑 —— 有意分歧，双方在文档里各记一笔。

### tcl_interactive

```
% set tcl_interactive
1
```

rtcl REPL 与 web REPL 启动时都执行 `set tcl_interactive 1`
（`Interp::new()` 默认 0，script 模式保持 0）。

### history

tclsh 交互态有完整 `history` 命令（`history` 列事件表、`history keep N`、
`history event N`、`history redo` 等），且可编程（`[llength [history]]`）：

```
% history
     1  set tcl_interactive
     2  puts "INTERACTIVE=$tcl_interactive"
...
```

rtcl 现状：CLI 上下键历史由 rustyline 提供，`.history` 点命令打印编号
列表（只读）；web 用输入框上下键。**差距**：没有可编程的 `history`
Tcl 命令、无 `!` 展开（tclsh 本身也没有 `!` 展开）。作为点命令扩展面
保留，待需求出现再补内置命令。

## rtcl 侧实现索引

- `crates/rtcl-cli/src/repl.rs` — `ReplInput`（多行组装）、提示符选择、
  控制流文本、exit 码映射；9 个单元测试。
- `crates/rtcl-cli/src/main.rs` — `run_repl`/`eval_repl_line`（tclsh 式
  输出与退出码）、`.history`。
- `crates/rtcl-wasm/src/lib.rs` — `RtclHandle::is_complete`。
- `crates/rtcl-wasm/www/index.html` — web REPL（is_complete 接 interp、
  同一套 parity 输出规则）。
- `crates/rtcl-wasm/test.js` — is_complete 的 node 侧回归（24/24）。

## 已知差距（截至本文）

1. `history` 内置命令（可编程事件表）未实现——点命令 `.history` 替代。
2. tclsh 的 `tcl_readline`/`!` 历史展开不存在（tclsh 8.6 亦无）。
3. script 模式 `exit N` 的退出码（rc=1 vs tclsh rc=N）是文件模式语义，
   归 judge 语料管，REPL 侧已对齐。
