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

- 新增 feature `regexp-spencer`（默认关闭，验证后转默认）。
- `regexp_cmds` 在 feature 开启或 `RTCL_REGEX_ENGINE=spencer` 时
  路由到新引擎；其余保持现状（避免未完成期回归）。
- fast 引擎保留为"无歧义模式"的加速路径（M4 再评估）。

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
  parse,find,is_match}`；`regexp-spencer` 默认关闭）。
- **M1 完成**：ARE 解析器（`parser.rs`）覆盖字符/类/量词（含惰性/
  `{m,n}`）/交替/捕获/非捕获/lookahead（解析层）/断言/转义
  （`\u\U\x`、八进制/回引启发式、`\b\B\a`、`\z` 拒绝）/
  `***=`/`***:`/内嵌选项/expanded；错误码与消息按 oracle 表。
- **M2 完成**：NFA（`nfa.rs`）+ 整体匹配（`matcher.rs`，POSIX
  leftmost-longest / SHORTER）；`specs/regex/spencer-overall.py`
  对 tclsh 8.6.17 的随机 4000 例**整体 span 差异 = 0**（默认选项，
  不含回引/lookahead）。
- **M3 待做**：捕获 dissect（POSIX 子匹配/迭代/回引）与
  `-inline/-indices/-all` 接线；M4：lookahead/LACON、回引执行、
  BRE/ERE/expanded 全量；M5：默认路由与 go/no-go。
- 验证命令：
  `cargo test -p rtcl-vm --lib --features regexp-spencer regexp_spencer`
  （7/7）；`specs/tclmem.sh 'python3 specs/regex/spencer-overall.py'`
  （4000/4000）。

## 8. 工程约定

- 新引擎不 panic：所有失败走 `Error { code, message }`；
  编译空间上限（Spencer `REG_MAX_COMPILE_SPACE` 语义）用预算参数。
- 每里程碑独立提交；`regexp-spencer` 默认关闭直至 M5。
- 测试三层：
  1. 单元（lexer/parser/NFA/matcher 每层）；
  2. oracle 定向（新增 `specs/regex/*.tcl` 语料）；
  3. 全量差分（`specs/regex/run.sh`）+ judge/probes 回归。
- 所有 tcl 侧运行经 `specs/tclmem.sh`（512M 护栏）。
