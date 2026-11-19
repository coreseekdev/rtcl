# rtcl 模块化路线图（wasm dispatcher 面收窄）

> **状态**：定稿（2026-11-19；同日 clock+package 落地）。本文是「执行
> 模块化」的路线交付：现状盘点 + 里程碑链 + **侧模块 ABI v1 规范（正
> 文）** + 候选拆分清单（clock+package 已按方案 a 落地，余项状态在表）
> + 发布配方矩阵（size 轨数字由 `build.sh --size` 供给）。
> 方法层来源：feat/rtcl-wasi 瘦身四招（依赖声明层裁剪 / size profile
> / 服务宿主化 / imports 最小化）——本路线取 ①③ 为主产线、②（size
> profile）为增量；结果层不综合（master 产物已同时比 wasi 更小更强：
> pkg 2.01MiB < wasi 2.25MB（10vicesimustertius 实测），且 99 命令 +
> 字节码 VM vs 树走查）。
> 关联：`HANDOFF.md` §10quinter（file/io 评估定调）/§10septem（json
> 外置）/§10tertio（dynmod ABI v2）/§10tredecimus（spencermod）/
> §10decimus（OPFS）/§10undecimus（regexp+encoding 外置）/§10sexter
> （CommandFunc 表定调 + wasm-opt）。

## 1. 现状盘点（本体构成）

尺寸口径：raw = cargo 产物（wasm32-unknown-unknown release）；pkg =
wasm-bindgen + wasm-opt -Oz 之后；gzip = pkg 压缩面。

| 构成面 | 尺寸 | 定性 | 处置 |
|---|---|---|---|
| **CommandFunc 函数指针表** | **1.33MiB（58%）** | 结构性 retain：elem 段仅 877B（函数索引表），1.33MiB 是只能经 fn 指针间接到达的命令函数体——LLVM 看不到直接调用点，无法内联/常量折叠/去重。**尺寸现象而非死代码**（热路径不踩表：Tier-1 命令在 vm_exec 已编译为操作码；V8/wasmtime 对 call_indirect 做 inline cache） | 不可死剥离。限制内联已是现状（profile 满配：lto/codegen-units=1/panic=abort + wasm-opt -Oz）；薄包装合并（cmd_fcopy→chan_copy 式 delegator）量级 <20KiB，待真机回归护栏 |
| **file+io（HostFs 服务化）** | 净重 **164KiB** raw（core 2,551,625 → 2,715,288） | **不可拆**——OPFS 文件面是 dispatcher 的功能（网页里 open/gets/puts/file/glob/source 真实可用），不是死重。尺寸不是外置动机（6% 净重），动机是功能诚实性；本轮已完成形态升级：通道协议面留原生（chan/fconfigure/缓冲/反射——通道状态在解释器里），数据面走 HostFs 服务 trait（48 处 std::fs 直连点收敛；native=NativeFs，wasm=JsFs→OPFS/SAB 桥） | 保留。嵌入方不要 file/io 时 feature 已可独立关（`(std,dynmod)` 可编译） |
| **已外置 JS 面** | ~0（wasm 内已剔除） | regexp（regexpmod.js 语义子集 + spencermod.wasm 全集）、encoding（encodingmod.js，86 名宽表 TextDecoder/TextEncoder）、json（jsonmod.js，数字原文保留 + Tcl 序列化）、**clock+package（clockpkgmod.js，2026-11-19 落地）** | 完成。命令面经 dynmod 注入，wasm 初始计数不变 |
| **侧模块 spencermod.wasm** | **120KB**（独立构建） | 本体小 + 热路径侧载的样板：native 同源 Spencer 引擎，数值 ABI + envelope 交接；regsub/捕获/选项全集对齐 native | 完成（§3 规范正文以此为参照实现） |
| **clock + package** | 净重 **85KiB** raw（perf 轨剥离实测 −85,307） | ~~候选拆分~~ → **落地（2026-11-19，方案 a = JS dynmod）**：wasm 的 clock 本是 UTC-only stub（libc 不进 wasm），package 是 pkgIndex 面——JS 面（clockpkgmod.js）同面补回并**升级**：本地时区 + IANA `-timezone`（Intl tzdata 求值）+ 完整 D-24/D-25 行为矩阵移植 | 完成（§4 记录定调；计数面零变化） |
| **stdlib proc** | 0（wasm） | ui/http/log/rules 等是 Tcl 源码（`load_*` 注入），非 wasm 代码 | 非议题 |
| **C-float / C-lazy** | — | float 快车道（feature 默认 off，off 轮零成本）/ lazy-string（wasm 默认 on） | 已落地（HANDOFF §10vicesimusprimus / §10vicesimus） |

本体数字链（perf 轨，wasm-opt 后，2026-11-19 clock/package 剥离后）：
raw **2.61MiB**（2,739,147）→ pkg **1.94MiB**（2,031,839）→ gzip
**738KiB**（756,245）；历史链：raw 2.69MiB → pkg 2.01MiB（2,105,384）/ gzip
765KiB（剥离前 @ 10vicesimusquartus）；wasm 命令计数 **99**（无 socket
[net 门控]、无 exec [wasm 子集本就不含]；clock/package 由 clockpkgmod
JS 面补回，`info commands` 逐字不变；load 后 101、unload 回 99）。

## 2. 已完成的模块化里程碑链

```
regexp-lite 剔除（97KiB raw，2.84→2.74MiB）
  → regexpmod.js            JS 语义子集（dynmod 注入 ::regexp/::regsub）
  → spencermod.wasm         native 同源 Spencer 全集侧模块（120KB，侧模块样板）
  → encodingmod/jsonmod     encoding 86 名宽表 + JSON 全面（JS dynmod）
  → dynmod ABI v2           列表值单词直传（join 分隔符协议退役）
  → HostFs                  file/io 数据面宿主服务化（OPFS/SAB 两形态）
  → clockpkgmod.js          clock+package 剥离（85KiB raw）+ JS 同面补回
                           （本地时区/IANA -timezone 升级，2026-11-19）
```

配套机制：`load`/`unload` 宿主预实例化注册表（wasm 无 dlopen——
`load` 是同步命令而 instantiate 异步）；`register_module` 默认 v2
（`__dynmodv2:` 前缀逐词透传，任意 Unicode/空白/\u0001 整词保真）；
`set_fs`/`set_output_handler`/`set_clock` 服务宿主化同款缝。

## 3. 侧模块 ABI v1 规范（正文）

侧模块 = 独立构建的 wasm32-unknown-unknown cdylib，经 dynmod 挂线为
Tcl 命令面。参照实现：`crates/rtcl-regexp-spencer-side`（spencermod，
产物 120KB）。新侧模块按本节逐条实现；协议版本号 = 各模块 `_abi()`
返回值（spencermod = 1）。

### 3.1 数值 C ABI 五函数模式

全部导出为**纯数值签名**（指针/长度/u32 返回，无字符串 ABI、无
wasm-bindgen）——JS 胶水负责字节 ↔ 线性内存搬运：

```wat
<name>_abi() -> i32                 ;; 协议版本（envelope/flags 编码变了才 bump）
<name>_exec(in_ptr, in_len) -> i32  ;; 单次调用入口：读输入 envelope、执行、
                                    ;;   写结果 envelope；返回 0（成功/失败由
                                    ;;   tag 区分——编译错误不是 trap）
alloc(len) -> ptr                   ;; 输入缓冲（胶水先 alloc、写 envelope、再传指针）
res_ptr() -> ptr                    ;; 结果 envelope 指针（exec 后读取）
res_len() -> u32                    ;; 结果 envelope 长度
```

- 五函数是基座；域面入口可按需加（spencermod 的 `about` 即第二个
  exec 形入口——同 envelope 协议，`regexp -about` 分析面）。
- 内存模型：单线程 JS 宿主，模块侧静态缓冲复用即可（spencermod：
  `static mut INPUT/RES: Option<Vec<u8>>`——alloc clear+resize，
  res 每次 exec 重建）。多会话并发不在 v1 面（JS 单线程）。
- 入口只返回 0：失败也走 tag 1 envelope（错误是普通值，不 trap——
  与 JIT ABI v2 的错误哨兵同哲学：`catch` 语义要求错误可携带）。

### 3.2 envelope 约定

字节流 = **LE u32 记录表**（前置长度 + 载荷，逐记录串联）：

```
输入 envelope：u32 nargs | per arg: u32 len + UTF-8 bytes
  标量参数一律十进制文本（spencermod：flags = 位或的十进制串、
  start = 字符偏移十进制串、all = 0/1）——避免第二种编码，胶水
  String() 即可组包。
输出 envelope（tag 打头）：
  tag 0（成功）：u32 0 | 域面记录（如 spencer exec: u32 ngroups |
    u32 nmatches | per match: ngroups × 2 × i32 字符起止，端点独占，
    未参与 = -1,-1）
  tag 1（错误）：u32 1 | u32 codeLen + code | u32 msgLen + msg
    （code = native 错误码名如 REG_*；msg 与 native 错误**同文**）
```

- 跨度/索引一律**字符单位**（引擎内部 span 即字符索引的直接产物；
  用户面 `-indices`/`string range` 皆字符单位，侧模块直出字符免宿主
  二次换算）。
- 错误同文原则：msg 逐字节复刻 native 错误消息（spencermod 编译错误
  前缀 `couldn't compile regular expression pattern:` 由胶水补），
  差分门（judge/probes）才能对侧模块与 native 双向适用。

### 3.3 崩溃面

- no_std 侧模块：致命错误 `loop {}`（wasm 不可恢复，胶水层以
  超时/不回包兜底——v1 不做恢复）。
- std 侧模块（spencermod 同款）：`panic = "abort"`——panic 即 trap，
  胶水 instantiate 层 catch 后走回退路径（worker：回退 regexpmod.js
  语义子集）。

### 3.4 构建配方

```toml
# crates/<name>-side/Cargo.toml —— 独立 workspace（不入根）：
[lib]
crate-type = ["cdylib", "rlib"]   # rlib = 宿主集成测试可链接（快照护栏）
[workspace]                        # 空表 = 脱离根 workspace（跨路径依赖
                                   # 需要独立包元数据）
[profile.release]
opt-level = "z"                    # 尺寸优先
lto = true
panic = "abort"
strip = true
```

```sh
cargo build -p <name> --target wasm32-unknown-unknown --release
# 产物尺寸数字链（spencermod 实测）：raw（cdylib）→ strip 后 → 入库
# 120KB。侧模块不经 wasm-opt（引擎小，LLVM z 档已到位；如需可补 -Oz）。
```

### 3.5 挂线协议（dynmod v2 + JS 胶水）

```
Tcl 面：load <name>（或 load <name>.wasm）→ 安装 ::<name>::<export>
        unload <name> → 全部拆除
胶水面：setupXxxMod(handle, opts) =
        ① fetch/instantiate（opts.wasmBytes 供 node 测试链注入；
           失败 → 返回 false，调用方回退 JS 子集实现）
        ② <name>_abi() 版本核对（胶水已知版本 ≠ 模块 → 回退）
        ③ register_module('<name>', glue) → load + wrapper proc 安装
glue 每命令：wordsOf（v2 逐词透传，v1 join 形态兼容）→ TextEncoder
        组 envelope → alloc 写入 → exec → res_ptr/res_len 读出 →
        DataView LE 解码 → 返回字符串（错误 throw，Tcl 面成 error）
```

wrapper proc（Tcl 侧）负责选项解析/upvar 变量写入/输出形组装——
语义 wrapper 与数值引擎分离，胶水不含选项逻辑（spencermod 的
`SPENCER_SETUP_TCL` 即此层）。

## 4. 候选拆分面清单（按收益/风险排序）

| # | 候选 | 收益 | 成本/风险 | 状态 |
|---|---|---|---|---|
| 1 | **clock+package（85KiB raw）** | wasm 半死面剔除：clock stub（UTC-only）与 package pkgIndex 面在网页 dispatcher 无真实用户 | 计数面风险 → **JS dynmod 形态消除**：worker 无条件装载补回同面命令 | **落地（2026-11-19，用户拍板方案 a）**：wasm 剥离 + www/clockpkgmod.js 补回，`info commands` 面 99=99 逐字不变；JS 面行为矩阵自 native clock.rs/package.rs 逐条移植（118 例 node 差分 + 真机 K/P 区 21 例），并升级本地时区/IANA `-timezone`。require 的 pkgIndex 扫描在 wrapper proc（Tcl 面编排 source——JS 命令回调无法重入解释器），registry 状态在 JS 模块 |
| 2 | CommandFunc 1.33MiB | —（结构性） | 不可死剥离（§1 定调）；薄包装合并 <20KiB 需真机回归护栏 | 挂账不动 |
| 3 | file+io 164KiB | —（**不可拆**） | OPFS 文件面是 dispatcher 功能：`open/gets/puts/file/glob/source` 在网页真实可用（fs-opfs 41/41 真机）；服务化已完成（数据面 HostFs、协议面原生） | 不拆（理由见 §1） |
| 4 | C-float/C-lazy | 已落地 | — | — |

## 5. 发布配方矩阵

| 配方 | features | raw | pkg | gzip | 适用场景 |
|---|---|---|---|---|---|
| **full**（默认） | std,file,io,dynmod,json（clock/package 已剥离→JS 注入） | 2.61MiB（2,739,147） | **1.94MiB**（2,031,839） | 738KiB（756,245） | 网页 dispatcher 完整面（99 命令〔clock/package 由 clockpkgmod 补回〕+ OPFS + spencermod/encodingmod/jsonmod/clockpkgmod 注入） |
| **slim** | std,file,io,dynmod | 2.59MiB（10quinter 实测 2,715,288，历史口径含 json） | ~1.8MiB（估） | — | 嵌入变体：无 json（走 JS 注入）；OPFS 保留 |
| **core** | std,dynmod | 2.43MiB（10quinter 实测 2,551,625，历史口径含 json） | — | — | 极简面：嵌入方自带 fs/console 服务实现 |

（slim/core 的现 HEAD 重测挂账——需临时改 Cargo.toml feature 组合，
非本轮范围；表中数字为 10quinter 时的同口径链。full 行 = 2026-11-19
clock/package 剥离后实测：raw −85,307 / pkg −73,545 / gzip −27,387。）

**size 轨**（wasi 招②增量）：`./build.sh --size` 产出第二产物
`rtcl_wasm_bg_size.wasm`（profile `wasm-size`，opt-level "s"）。
实测数字链（2026-11-19，clock/package 剥离后；括号内为剥离前
10vicesimusquartus 口径）：

| 轨 | raw | pkg（bindgen + wasm-opt -Oz） | gzip |
|---|---|---|---|
| perf 轨（release, opt3） | 2,739,147（2,824,454） | 2,031,839（2,105,384） | 756,245（783,632） |
| size 轨（wasm-size, opt-s） | 2,324,340（2,390,402） | **1,675,543（1,731,902）** | **653,116（674,592）** |
| 轨内 Δ | −15.1% | **−17.6%** | −13.6% |
| 剥离 Δ（size 轨） | −66,062 | **−56,359** | −21,476 |

**运行时代价 −40-50%**（wasi 线 opt3→opt-s 同族实测参照）——只用于
尺寸敏感的嵌入分发；dispatcher 真机面恒走 perf 轨。JS 胶水两轨同面
（同 exports/imports），部署 = 同一份 `rtcl_wasm.js` 配 `*_size.wasm`
换名即用。
