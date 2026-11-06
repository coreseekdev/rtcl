# 正则引擎语义差异评估（是否引入 sregex 类流式引擎）

日期：2026-11-06
范围：`regexp` / `regsub` 的 Tcl 8.6.17 语义对齐；评估"Rust 复刻
sregex 类高性能流式引擎"的收益与代价。

## 1. 结论摘要

- **不建议为语义对齐引入 sregex 类引擎**。sregex（openresty/sregex）
  是 **Perl 兼容、非回溯、字节流** 引擎；它带来的语义是
  leftmost-first（Perl 风格），与 Tcl 的 Spencer 引擎
  （POSIX leftmost-longest + SHORTER 惰性模式 + POSIX 子匹配规则）
  在三类核心差异上**全部不一致**。复刻它既不能修 D-20，也不能修
  交替/子匹配语义，反而会丢掉 Unicode 与反向引用/环视支持。
- 实测影响面：随机 4000 例语料 **523 例（13.1%）** 与 tclsh 不同，
  其中 **惰性/SHORTER 约 90%（455 例）**、**leftmost-longest 约 9%
  （47 例）**、**POSIX 子匹配 4%（21 例）**。现有 judge/探针全绿并
  不代表语义对齐——语料没有覆盖歧义形状。
- 真正能收口的是 **Spencer 语义**（移植其引擎，或给现有 BT 引擎
  实现 POSIX 最长/SHORTER/子匹配规则）。流式能力与 Tcl 的整串
  API 关系不大，可作为独立性能项目，而非正确性项目。

## 2. 当前实现与语义模型

| 层 | 现状 | 语义 |
|---|---|---|
| fast 引擎 | Rust `regex` crate（纯 Rust，线性时间） | **leftmost-first（Perl）** |
| BT 引擎 | `regexp_bt.rs`（~1k LOC 回溯） | 覆盖反向引用/环视/`\y\m\M`；偏好也是 Perl 风格 |
| 路由 | Tcl-Unicode 类、fast 编译失败 → BT | 其余全部走 fast |
| 预处理/校验 | `validate_are`、`build_pattern`、`expand_unicode_escapes` | Tcl ARE 语法/转义面（本批修了 `\u\U\x`、`\z`、`\b\B\a`、反向引用号） |

Tcl 8.6.17 的引擎是 Henry Spencer 的 `regcomp.c`/`regexec.c`（参考树
`.refer/tcl/generic/`，约 11.3k 行 C，含 NFA/locale）：

- 默认 **POSIX leftmost-longest**：同一位置取最长整体匹配
  （`a|ab` 在 `ab` 上得 `ab`，不是 `a`）。
- 任一非贪婪量词会把语法树标上 `SHORTER` → `REG_USHORTEST`：
  整体匹配偏好翻转为**最短**（Spencer 的 dissect/iteration
  最短优先），但贪婪量词仍先按自身贪婪展开（`(a+)(a*?)` 仍是
  `aaa`）。
- 子匹配按 POSIX 规则消歧（重复组取最后一次迭代等；
  `(a*)+` 在 `a` 上整体 `a`、组 `{}`）。
- 转义表是 Tcl ARE 的（`\b` 是 backspace、`\B` 是反斜杠、
  `\y` 才是词界、`\z` 在 8.6.17 非法、`\1..\9` 必须引用已闭合组）。

## 3. 实测影响面

复现：`specs/tclmem.sh 'bash specs/regex/run.sh'`

### 3.1 定向语料 `specs/regex/semantics.tcl`（759 例）

```
cases=759 diff=62 (8.2%)
  lazy (SHORTER mode): 48
  leftmost-longest vs first: 14
```

代表例：

| 模式 @ 串 | tclsh | rtcl |
|---|---|---|
| `a|ab` @ `ab` | `ab` | `a` |
| `(a+?)(a*)` @ `aaa` | `a a {}` | `aaa a aa` |
| `(a*?)(a*)` @ `aaa` | `{} {} {}` | `aaa {} aaa` |

### 3.2 随机语料 `specs/regex/random-4000.tcl`（400 模式 × 10 串）

```
cases=4000 diff=523 (13.1%)
  lazy+alternation: 240
  lazy (SHORTER mode): 215
  leftmost-longest vs first: 47
  POSIX submatch/other: 21
```

- **惰性/SHORTER 家族**：455/523（87%），包括纯贪婪但重复组
  捕获歧义的形状（`(a*)+`、`(a+)+`）。
- **整体最长 vs 首个**：例如 `(a|ab)(b){1,2}`、`(a|ab)*`，
  即使无惰性量词也受影响。
- **子匹配消歧**：整体匹配相同但捕获不同（`(a*)+` @ `a`：
  tclsh 捕获 `{}`，rtcl `a`）。
- 0 例编译错误差异（本批转义/校验修复后）——错误面已基本对齐，
  差异纯粹是**匹配偏好**。

### 3.3 现有门禁的覆盖盲区

- judge 88/88、`gen_regexp`/`gen_regexpComp` 的 562 行语料全部通过；
  matching-deep 只有 1 条惰性探针（D-20）。
- 说明现有测试**没有覆盖**交替歧义/重复组捕获形状；上述 13.1%
  是"未测到的行为面"，而不是新引入的回归。

## 4. sregex（openresty/sregex）评估

事实（README/仓库）：

- 纯 C、零依赖、**非回溯 NFA/DFA**（Thompson yes/no + Pike 子匹配 +
  x86_64 JIT），面向**大数据流**的流式匹配（分块输入、`SRE_AGAIN`、
  子匹配也可流式）。
- **Perl 5 语义**（README 明示 Perl-compatible；`\A/\z/\b/\B` 均为
  Perl 含义），支持贪婪/非贪婪、`{}` 量词、捕获、大小写不敏感。
- **仅 octet 模式**（"no multi-byte character encoding love"）；
  语法表没有反向引用、环视、`\m/\M/\y/\Y`、POSIX 字符类。
- 以 PCRE/Perl 测试套件为回归基线。

Rust 复刻它的语义影响：

| 维度 | sregex 类引擎 | Tcl 需要 | 影响 |
|---|---|---|---|
| 匹配偏好 | leftmost-first（Perl） | leftmost-longest（Spencer） | 修不了 47 例交替差异，`a|ab` 仍错 |
| 惰性量词 | 每量词 Perl lazy | 任一 lazy → 全局 SHORTER | 修不了 455 例惰性差异 |
| 子匹配 | Perl 规则 | POSIX 规则 | 修不了 21 例子匹配差异 |
| Unicode | octet-only | UCS-2/UTF-8 + Tcl 折叠 | 需重写字符层或放弃 Unicode 语义 |
| 反向引用/环视 | 不支持 | ARE 支持 | 仍需 BT 引擎兜底 |
| 流式 | 分块/有界内存 | Tcl 是整串 API | 仅对未来的大流 API 有价值 |

即：**复刻 sregex 对现有 523 例差异的修复数为 0**；它提供的是
"线性时间 + 流式 + JIT"的工程属性（Rust `regex` crate 已提供线性
时间，流式只对超大输入有意义）。若目标是性能/流式，应单独立项并
先给出 GB 级流式 `regexp` 的产品需求与基准；不应与语义对齐混在
一起。

## 5. 可选路线

### A. 现有双引擎 + 精确化（推荐起点）

- 保留 fast 引擎处理"无歧义"模式（无 `|`、无惰性、无重复组捕获），
  其余路由到 BT。
- 给 BT 引擎实现 Spencer 偏好：整体 longest / SHORTER、贪婪与惰性
  优先级、POSIX 子匹配消歧。
- 代价：BT ~1k LOC + 约 0.5-1k LOC；风险是子匹配消歧细节多，需用
  oracle 语料 fuzz 验证；fast 路径继续承担热点性能。
- 收益：覆盖全部 523 例差异族，且不引入新引擎/依赖。

### B. Rust 移植 Spencer（语义最稳）

- 参考 `.refer/tcl/generic/regcomp.c`/`regexec.c`（约 11.3k LOC C，
  含 NFA/locale）；Rust 移植预计 8-12k LOC + Tcl Unicode/折叠胶水。
- 优点：与 tclsh 同源，SHORTER/子匹配/ARE 转义天然一致；可先作为
  唯一引擎保证正确，再逐步把安全子集交回 fast 引擎。
- 代价：工作量大（周级）、需长期维护两套；但比"sregex 复刻"更贴合
  parity 目标。
- 风险：8.6.17 与参考树版本差异（`\z` 等）需以 oracle 探针钉死。

### C. 引入 sregex 类第三引擎（不推荐）

- 只在"大流式匹配"成为独立产品需求时考虑；语义上不解决任何现有
  差异，且会引入 Perl vs Tcl 的新分歧、octet/Unicode 双模型、
  反向引用/环视缺失。
- 工作量：跨语言复刻 + Tcl 语法/Unicode 扩展 + 与 BT 的路由，远大于 B。

### D. 标签 NFA 的 POSIX 引擎（折中）

- 2-4k LOC 的 tagged-NFA（RE2 风格）实现 leftmost-longest + POSIX
  子匹配标签；SHORTER/惰性需额外优先级层或回退 BT。
- 比 B 小，但"精确等价 Spencer"需大量 oracle 差分；适合作为 B 的
  中间形态。

## 6. 建议

1. **先做 A**：把歧义形状路由到 BT 并实现 Spencer 偏好；用
   `specs/regex/run.sh` 的 4000 例作为收敛指标（目标：13.1% → 0）。
2. 若 A 的子匹配细节反复不收敛，再启动 **B**（移植 Spencer），并把
   fast 引擎降级为"可证明无歧义"的加速路径。
3. **sregex 复刻不立项**，除非出现明确的 GB 级流式正则需求；届时
   单独评估"流式 API + 内存上限 + 吞吐"指标，而不是拿它当语义修复。

## 7. 复现与资产

- `specs/regex/semantics.tcl` —— 定向 759 例。
- `specs/regex/random-4000.tcl` —— 随机 4000 例（seed 20261106）。
- `specs/regex/gen-corpus.py` —— 随机语料生成器（确定性）。
- `specs/regex/analyze.py` —— 差异计数与分类。
- `specs/regex/run.sh` —— 一键跑 tclsh vs rtcl（经 512M 护栏）。
- 相关台账：D-19（转义，已 FIXED）、D-20（惰性偏好，OPEN）、
  D-18（lookbehind 路由）、D-17/21（锚点）。

本批已顺带收口的转义差异（见 D-19）：`\u`(1-4 hex)/`\U`(1-8)/
`\x`(1-2) 接受、`\x{...}` 拒绝、`\z` 拒绝、`\b`=backspace、
`\B`=反斜杠、`\a`=bell、`\1..\9` 只能引用已闭合组
（`REG_ESUBREG`）。
