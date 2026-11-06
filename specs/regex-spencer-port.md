# 正则引擎 Spencer 移植项目（B 路径）

日期：2026-11-06
决策：按路线 B 正式立项——把 Tcl 8.6.17 使用的 Henry Spencer 正则
引擎移植为纯 Rust 实现，作为 `regexp`/`regsub` 的语义基准引擎。
评估依据见 `specs/regex-engine-eval.md`（sregex 类 Perl/流式引擎不能
修复 D-20/D-46 的 523/4000 差异）。

## 1. 目标与非目标

目标：

- 语义与 tclsh 8.6.17 **逐字节/逐捕获一致**：POSIX leftmost-longest
  整体匹配、`SHORTER` 惰性模式（任一非贪婪量词 → 整体最短）、
  POSIX 子匹配消歧、反向引用、前向断言（lookahead）、ARE 转义/
  字符类/内嵌选项/`***=` 字面。
- 纯 Rust、零外部依赖、可在 wasm32/no_std（alloc）构建；
  不依赖 libc/iconv。
- 作为正确性基准：先覆盖全部 4000 例语义语料，再逐步把可证明
  无歧义的子集交回 fast 引擎（Rust `regex` crate）做性能加速。
- 可复现的差分门禁：`specs/regex/run.sh`（tclsh vs rtcl）与新增
  的引擎单元/差分测试。

非目标（首版）：

- 不复刻 tclsh 内部 `regcomp`/`regexec` 的 C 数据结构与内存池，
  只复刻**可观察语义**。
- 不做流式/增量匹配（Tcl 的 regexp API 是整串）；若未来出现大流
  需求单独立项。
- 不改变 `string` 的码点语义策略（D-27）；Spencer 引擎内部按
  Tcl_UniChar（UCS-2）语义处理，边界转换在适配层统一。

## 1.5 现成引擎调研（2026-11-06）

结论：**没有现成的 Rust 引擎具备 Tcl/Spencer 语义**，B 路径的自研
移植成立。

| 候选 | 语义 | 是否可用 |
|---|---|---|
| Tcl 源码 `regcomp.c`/`regexec.c` | Spencer：POSIX longest + SHORTER + POSIX 子匹配 + ARE | C 参考实现（BSD），仅可移植 |
| glibc POSIX `regcomp/regexec` | POSIX longest/子匹配，但无 ARE/惰性/回引/环视 | C、非 wasm、语法面不足 |
| RE2（C++）/ Go `regexp`（Longest） | leftmost-longest 但 Perl 语法子集，无 ARE/惰性 SHORTER/回引 | C++/Go，非 wasm 可嵌入 |
| Rust `regex` / `regex-automata` | leftmost-first（Perl），无 POSIX longest/回引/环视 | 语义不符（当前 fast 引擎） |
| Rust `fancy-regex` / `onig` / `pcre2` | Perl/PCRE 语义（回溯或绑定 C） | 语义不符、非纯 Rust/非 wasm |
| Rust `posix-regex`（Redox） | POSIX 正则**解析器**（无匹配引擎） | 不可用 |
| openresty/sregex（C） | Perl 兼容、octet-only、无回引/环视、流式 | 语义不符（见评估文档） |

因此选择以 Spencer C 源码为规范、Rust 重实现（本文件）。

## 2. 参考实现与许可

- 参考：`.refer/tcl/generic/{regcomp.c,regexec.c,regc_lex.c,
  regc_nfa.c,regc_color.c,regc_cvec.c,regc_locale.c,rege_dfa.c,
  regerror.c,regex.h,regguts.h}`（约 11.3k 行 C）。
- Spencer 代码为 BSD 风格许可（源码头保留版权声明即可），与本仓
  BSD-2-Clause 兼容。移植文件头保留原版权与"Rust port"说明。
- oracle 为系统 `tclsh` 8.6.17；`.refer` 版本较新（如 `\z`），差异
  一律以 oracle 探针钉死（见 D-19）。

## 3. 架构

```
crates/rtcl-vm/src/interp/commands/regexp_spencer/
├── mod.rs        # 公共 API：Options/Prog/Error/exec/find
├── ast.rs        # subre 树：op('='/'b'/'('/'./'|'/'*')、flags、min/max、subno
├── charset.rs    # 字符集：范围 + Tcl Unicode 谓词 + 大小写折叠
├── parser.rs     # ARE 词法/语法（regc_lex.c + regcomp.c 的 parse 部分）
├── nfa.rs        # Thompson NFA（状态/弧/断言/LACON/回引标记）
├── matcher.rs    # 整体匹配（leftmost-longest / SHORTER）与捕获 dissect
└── tests.rs      # 单元 + oracle 对照
```

与现有引擎的关系：

- 新增 feature `regexp-spencer`（已并入 `rtcl_std` 默认开启）。
- `regexp_cmds` 默认路由到新引擎；`RTCL_REGEX_ENGINE=fast` 退回旧
  fast/BT 路径（差分/应急）。
- fast 引擎保留为旧语义对照路径（不再是默认）。

## 4. 语义清单（验收口径）

1. 整体匹配：同一左端位置取最长（默认）；树含 `SHORTER` →
   最短；`LONGER/SHORTER/MIXED` 传播按 Spencer `COMBINE/UP`。
2. 子匹配：POSIX 规则——较早/较外层子表达式优先最长，重复组取
   最后一次迭代；`(a*)+` @ `a` → 整体 `a`、组 `{}`。
3. 贪婪/非贪婪：`* + ? {m,n}` 的 `?` 后缀；`{m}` 不产生偏好；
   `{m,n}` 即使 `m==n` 也产生偏好（Spencer 实测）。
4. 断言：`^ $ \A \Z \m \M \y \Y`；`-line/-linestop/-lineanchor`
   对新行敏感性的影响；`\b`=backspace、`\B`=反斜杠。
5. 字符类：范围、`[[:class:]]`、`[.coll.]`/`[=equiv=]`（按 Tcl
   支持面）、大小写不敏感、否定类。
6. 转义：`\u`(1-4)/`\U`(1-8)/`\x`(1-2)、八进制/回引启发式、
   `\d\D\s\S\w\W`、`\a\e\f\n\r\t\v`、`\cX`、`\Q..\E` 拒绝、
   `\p`/`\P` 拒绝（8.6 ARE 面）。
7. 回引：`\1..\9`（多位数按启发式），只能引用已闭合组。
8. lookahead：`(?=)`/`(?!)`；lookbehind 在 8.6 拒绝（REG_BADRPT）。
9. 选项：`***:`/`***=`、内嵌 `(?i c n m p s t w x b e q)`；
   非法选项 REG_BADOPT；expanded 模式空白/`#` 注释。
10. 错误：消息与 `REG_*` errorCode 与 tclsh 一致（复用现有
    validate 的 oracle 表，逐步迁移到新引擎）。
11. 索引/编码：`-indices` 的字符索引；Unicode 以 Tcl_UniChar
    代码单元为准（与 tclsh 一致；与 rtcl 现有码点策略的边界在
    适配层显式转换，D-27 另行决策）。

## 5. 里程碑与验收

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | 立项、模块骨架、feature 门控、API 定义 | `cargo test --features regexp-spencer` 可编译 |
| M1 | ARE 词法/语法 → AST；错误消息/REG 码 | 解析 oracle 语料（合法/非法）全一致 |
| M2 | NFA + 整体匹配（longest/SHORTER/断言） | 4000 例**整体匹配**（不含捕获）差异=0 |
| M3 | 捕获 dissect（POSIX 子匹配/迭代/回引） | 4000 例 `-inline` 逐字节差异=0；D-20 探针 |
| M4 | lookahead/LACON、回引、内嵌选项/expanded 全量 | judge 88/88；matching-deep 120/120 |
| M5 | 路由默认化、fast 引擎降级策略、wasm/性能回归 | 全门禁 + wasm node 24/24 + bench 无回退 |

收敛指标：`specs/regex/run.sh` 的 523/4000 差异在 M2 后应降至
仅捕获类（约 21 例 + 混合），M3 后归零。

## 6. 风险与对策

- **POSIX 子匹配消歧**是最难部分（regexec.c 的 dissect 递归）。
  对策：按 Spencer 的 `LONGER/SHORTER/MIXED/CAP/BACKR` 标志逐函数
  移植 `crevcondissect/crevaltdissect/creviterdissect/cbrdissect`，
  用 oracle 语料驱动，不自行发明规则。
- **UCS-2 vs 码点**：regexp 索引必须与 tclsh 一致（代理对算两个
  单元）；内部统一 `u16` 视图，输出时按 rtcl 的索引约定转换。
- **性能**：先正确后快；NFA 模拟 O(n·m) 可接受；若热点回归，
  再引入基于颜色类的 DFA 缓存（Spencer 原设计）或把安全子集切回
  fast 引擎。
- **版本差异**：`.refer` 为较新 Tcl；所有差异（`\z`、选项等）以
  oracle 探针钉死并记录台账。
- **许可**：保留 Spencer 版权头；新文件明示 Rust port 与修改。

## 7. 实施状态（2026-11-06）

- **M0 完成**：模块骨架/feature 门控/API（`regexp_spencer::{compile,
  parse,find,is_match}`；M5 后默认开启）。
- **M1 完成**：ARE 解析器（`parser.rs`）覆盖字符/类/量词（含惰性/
  `{m,n}`）/交替/捕获/非捕获/lookahead（解析层）/断言/转义
  （`\u\U\x`、八进制/回引启发式、`\b\B\a`、`\z` 拒绝）/
  `***=`/`***:`/内嵌选项/expanded；错误码与消息按 oracle 表。
- **M2 完成**：NFA（`nfa.rs`）+ 整体匹配（`matcher.rs`，POSIX
  leftmost-longest / SHORTER）；`specs/regex/spencer-overall.py`
  对 tclsh 8.6.17 的随机 4000 例**整体 span 差异 = 0**（默认选项，
  不含回引/lookahead）。
- **行为规格**：`specs/regex-spencer.allium`（23 条规则，oracle 值
  实测校准；`allium check` 0 error）作为 M2/M3/M4 的验收口径。
- **性能（M2/M2.5，见 §8）**：单趟起点跟踪 NFA + ASCII 位图 + 纯字面/
  前缀预过滤 + **颜色类 lazy DFA**（每状态一行的颜色转移表、状态/
  颜色上限、断言回退）。1 MiB 密集匹配类模式 65-116 MB/s（对
  `regex` 1.0-1.9×），全串无匹配 114-127 MB/s（5-6×），纯字面
  3.1 GB/s。search 预过滤/UTF-8 直读列为下一轮。
- **M3 完成（捕获 dissect）**：新增 `captures.rs`——concat 左最长
  （左子树 SHORTER 时最短）、alternation 按书写序、iteration 贪婪
  端点/重复组取最后迭代（每次 zap）；解析器按 Spencer 编译器改写
  `x{m,n}`(m>0)=`NoRecord(x{m-1,n-1}) x`，解释 `(a*)+` 组为尾部
  空迭代；回引按已捕获文本比较。`specs/regex/spencer-captures.py`
  对随机 4000 例 `-indices -inline` **差异=0**（默认选项，不含
  lookahead；lookahead 执行、BRE/ERE 属 M4）。
- **M4 核心完成**：lookahead 编译为 Lacon 子 Prog（NFA 闭包执行、
  DFA 自动禁用；捕获按非捕获处理）；反向引用走 `backtrack.rs`
  回溯执行（枚举 AST 终点 + 捕获快照 + 预算上限，空捕获回引按
  Spencer "matched trivially" 处理）。随机 4000 例与定向 759 例
  （含 lookahead/回引/\y/断言）捕获均 **0 差异**。
- **M4 完成（方言 + 命令面）**：BRE/ERE 语法（`(?b)`/`(?e)`/`***:`
  下的分组/量词/回引/转义/锚点位置规则/类内反斜杠字面/错误优先级）
  按 tclsh 探针钉死；`regexp -about` 的 re_info 标志面
  （UBACKREF/ULOOKAHEAD/UBOUNDS/UBRACES/UBSALNUM/UPBOTCH/UNONPOSIX/
  UUNSPEC/ULOCALE/UEMPTYMATCH/UIMPOSSIBLE/USHORTEST）按位序复刻。
  验证：BRE/ERE 定向探针 + 2500 pattern × 21 string 随机方言语料
  **0 差异**；`-about` 3000 pattern 随机语料仅余 2 例 `\y` 与
  `^$` 空串推理的极端不可命中标志（记录为已知边界）。
- **M5 完成（默认化）**：`regexp`/`regsub`（`-inline/-indices/-all/
  -start/-about` 与 `regsub` 替换）走 Spencer 引擎；`array names
  -regexp`/`lsort -regexp`/switch 等 `engine_is_match` 消费者同步。
  默认特征并入 `rtcl_std`；`RTCL_REGEX_ENGINE=fast` 可退回旧
  fast/BT 路径做差分。门禁：judge 88/88、probes 失败集与基线逐项
  相同（111 个存量非正则项）、4000+759 捕获 0 差异、单元测试 9/9。
- 验证命令：
  `cargo test -p rtcl-vm --lib --features regexp,regexp-spencer regexp_spencer`
  （9/9）；`specs/tclmem.sh 'python3 specs/regex/spencer-overall.py'`
  （4000/4000）；`specs/regex/dialect-probes.tcl` + `judge/run.sh`。

## 8. 性能结构（Rust 版）

Spencer 原引擎的性能来自四层结构：color/byte-tree 字符等价类 →
compacted NFA（小整数弧表）→ lazy DFA 缓存 → dissect 复用。Rust 移植
按同样分层推进，先保证语义，再逐层补性能：

### 已落地（M2）

- **单趟起点跟踪扫描**（`matcher.rs`）：所有起点线程同时推进，每个
  NFA 状态只保留最小起点（leftmost 优先）；不再每个起点重启模拟
  （旧版最坏 O(n²·m)）。
- **零分配热路径**：代际标记（stamp/start_of）+ 复用 stack/cur/seed
  Vec；每位置只做一次 epsilon/断言闭包。
- **ASCII 位图**：`CharSet` 对 0..127 预计算 4×u64 成员表；常见
  ASCII 文本不做逐项谓词扫描。
- **预过滤**：纯字面模式直接子串搜索；字面前缀 / 首字符集跳过
  不可能起点（断言可达时自动禁用首字符集）。

### 实测（1 MiB 文本；`examples/spencer_bench.rs`）

密集匹配（每 1-5 字节一个匹配，`find` 循环）：

| 模式 | Spencer Rust | Rust `regex` crate | 比值 |
|---|---|---|---|
| `a+` | 65 MB/s | 92 MB/s | 1.4× |
| `[a-z]+` | 82 MB/s | 155 MB/s | 1.9× |
| `\w+` | 81 MB/s | 146 MB/s | 1.8× |
| `(a\|ab)+` | 87 MB/s | 88 MB/s | 1.0× |
| `(a+?)(a*)` | 66 MB/s | 93 MB/s | 1.4× |
| `\s+` | 116 MB/s | 167 MB/s | 1.4× |
| 纯字面 `abab abab` | 3112 MB/s | 1310 MB/s | 0.42×（快路径） |

全串无匹配扫描（单次调用扫完 1MB，DFA 路径）：

| 模式 | Spencer Rust | Rust `regex` | 比值 |
|---|---|---|---|
| `(a\|b)*z` | 114 MB/s | 608 MB/s | 5.4× |
| `[a-z]*z` | 127 MB/s | 794 MB/s | 6.2× |

UTF-8→char 转换：2.4-3.3 GB/s（每次 regexp 调用一次性成本）。
DFA 缓存：颜色类 ≤64、状态 ≤4096；断言模式自动回退 NFA。

### 命令级实测（M5，1 MiB，`regexp -all`）

| 模式（匹配数） | tclsh 8.6.17 | rtcl Spencer |
|---|---|---|
| 纯字面 `abab abab`（104860） | 55 ms | 41 ms |
| `[a-z]+`（209720） | 38 ms | 79 ms |
| `\w+`（209720） | 39 ms | 51 ms |
| `(a+?)(a*)`（419440） | 66 ms | 64 ms |
| `[a-z]*z`（0，全串扫描） | 2 ms | 9 ms |

旧 fast 路径同机 `-all` 为 16 s / 32 s（逐匹配重扫前缀），已不默认。
关键修复：`collect_matches` 的字节→字符换算改成**单调游标**
（每个匹配不再 O(n) 重扫，整体 O(n)）；命令级 `CharCache` 复用
`Vec<char>`/字节表与 `Matcher` 的 DFA 缓存（`captures::exec_with`）。

### 待做（M2.5 后续）

1. **search 预过滤 DFA（Spencer `simpleFind` 两段式）**：先用
   `search` NFA 的 shortest 扫出可能起点区间，再对候选起点做 anchored
   longest/shortest；目标把全串无匹配从 114-127 MB/s 提到 ≥500 MB/s。
2. **UTF-8 直读 + 字面 memmem**：公共 `find(&str)` 对纯 ASCII/纯字面
   走字节路径（避免每次调用的一次性 `Vec<char>` 转换），只对非 ASCII
   /需要 Unicode 语义的输入转 chars。
3. **dissect 复用**：已完成——`Matcher` + `CharCache` 在命令级
   `-all` 循环复用 DFA/扫描缓冲。
4. **趋势目标**：密集匹配与 `regex` ≤1.5×（当前 1.0-1.9×）；
   全串无匹配 ≥500 MB/s；纯字面 ≥0.5× regex（当前更快）。
5. 内存上限：DFA 缓存按状态数/字节数封顶；编译空间预算对应
   `REG_MAX_COMPILE_SPACE`。

性能验收口径：正确性门禁（4000 例差异=0）优先；性能目标为常见类
模式 ≥1 GB/s、纯字面不低于 `regex` 的 0.5×、无病态回溯（NFA/DFA
路径无指数行为）。

## 9. 工程约定

- 新引擎不 panic：所有失败走 `Error { code, message }`；
  编译空间上限（Spencer `REG_MAX_COMPILE_SPACE` 语义）用预算参数。
- 每里程碑独立提交；M5 完成默认化（`RTCL_REGEX_ENGINE=fast` 可回退）。
- 测试三层：
  1. 单元（lexer/parser/NFA/matcher 每层）；
  2. oracle 定向（新增 `specs/regex/*.tcl` 语料）；
  3. 全量差分（`specs/regex/run.sh`）+ judge/probes 回归。
- 所有 tcl 侧运行经 `specs/tclmem.sh`（512M 护栏）。
