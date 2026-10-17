# JIT/WASM 内存管理与编译管线设计（路线 C）

> **状态**：设计定稿（2026-10-17）。JIT 实现仍处暂停（HANDOFF.md §5
> 裁决），本文是重启时的施工图：定架构、ABI、开关与验收口径，不改任何
> 现有代码。
> 关联：`HANDOFF.md` §6（里程碑与裁决）、`TODO.md`（fuel/epoch 缺口）、
> `docs/PRODUCTION.md` §3（wasm 部署现状）。

## 1. 问题与结论

**问题**：Tcl → WASM 编译/JIT 时，内存管理（GC）怎么办？要不要用 WasmGC？

**结论**：不需要 tracing GC，也不需要 WasmGC 管生命周期。Tcl_Obj 血统 =
引用计数 + 无环树结构；本实现 `Value` 是 8 字节 tagged word（i63 立即数 |
堆指针），堆对象 `ValueInner` 手写 RC + copy-on-write + 线程本地 freelist
（`crates/rtcl-vm/src/value.rs:317-492`）。RC 不需要根枚举/栈扫描，天然
免疫 tracing-GC-on-wasm 的最大陷阱（wasm 栈对程序不可见——MicroPython
要靠 `emscripten_scan_registers()` 把宿主栈涮出来扫，TinyGo 保守扫描已知
滞留垃圾）。

**定案 = 路线 C**：wasm 模块内 tagged-i64 立即数快车道 + 堆对象走
handle-arena 留宿主 RC + 字符串边界借 js-string-builtins 窄切口。
**全程不依赖 WasmGC**。

### 业界对照（2026-10 调研）

| 路线 | 代表 | 对 rtcl 的适配度 |
|---|---|---|
| 自带内存管理跑线性内存 | TinyGo（保守 mark-sweep）、Go（完整 runtime + stack maps）、Pyodide（CPython RC 原样跑）、**MoonBit wasm 后端（编译器优化 RC + incref/decref FFI 约定）** | ★ RC 天然可行；MoonBit 的 RC FFI 约定是现成设计模板 |
| 宿主 WasmGC | MoonBit wasm-gc、Dart、Kotlin/Wasm、wasm_of_ocaml、Guile Hoot | ✗ 丢「refcount==0 即时析构」语义（变量 trace、命令删除回调依赖）；WasmGC 无 finalizer/弱引用；Rust 宿主摸不到 GC 堆内对象 |

WasmGC 现状备忘：2025-09 随 Wasm 3.0 标准化；Chrome 119+ / Firefox 120+ /
Safari 18.2+ / Node 22+ 默认启用；Wasmtime 47（2026-07）默认开启但官方
明说性能尚不如 V8。硬限制：无 finalizer、GC 堆与线性内存隔离（引用不能
存进 linear memory）、无内部指针、字符串无标准表示。Go 弃用 WasmGC 的
原因（无内部指针、不保证非移动）同样适用于已有成熟值模型的运行时。

**长期跟踪项**：js-string-builtins（Chrome 131+/FF 134+/Safari 26.2+）
会改变字符串互通成本结构（§6）；WasmGC 列为「独立运行时成熟 + Rust→
WasmGC 工具链成熟后再评估」的远期实验。

## 2. 路线 C 总览

三原则：

1. **模块内 tagged word 与宿主 `Value` 的 8 字节形状同构**：bit0=1 → i63
   立即数；bit0=0 → arena handle（handle 0 = UNSET，与宿主哨兵对齐）。
   立即数穿越边界 = 纯拷贝（宿主 `from_bits` 零分配，不进 arena）。
2. **守卫失败 = 调宿主 full-semantics import**，不是错误、不是拒发。从
   M0 的编译级全有/全无演进为「编译仍全量、运行时快慢分流」——与
   HANDOFF §6 给 M1 定的 InternalRep 守卫一致，措辞修订记录在此。
3. **字符串内部表示不换**（UTF-8 SmallVec 不动），只优化边界（§6）。

**术语约定——「回退」有两个粒度**（勿混）：

- **守卫回退**（运行时，V8 deopt 同款）：已编译代码内守卫失败 → 调宿主
  full-semantics import 继续，proc 不离开编译态。类型层面的一切动态性
  （shimmer、bignum、字符串转数）都走这里，**不存在「动态类型推断不出
  就不能编译」的问题**。
- **编译拒绝**（编译期，LuaJIT NYI 清单同款）：proc 含「没有便宜守卫」
  的语义结构（trace 的变量、upvar 别名、运行时构造的 eval 字符串、
  errorInfo 记账不可省的段落）→ 整个 proc 留在解释器。tclsh 自己的
  tclCompile.c 就是「能编的编、编不了的留通用求值路径」的同一架构。
  两个粒度都以解释器为语义 ground truth，且帧状态共享（值在 arena、
  变量在宿主 frame），回退无需帧翻译。

与 WasmGC i31ref 的关系：同一思想（tag 区分立即数与堆引用，wasm_of_ocaml
/Hoot 都这么编码），但我们用 i64 而非 i31ref——不引入 WasmGC 依赖，还
白赚 32 位精度（i63 覆盖时间戳与绝大多数计数场景）。

## 3. ABI v2（在 M0 三函数上生长，breaking）

```wat
(global "rtcl" "fuel"  (mut i64))     ;; 步数预算：回边自减，宿主充值
(global "rtcl" "epoch" i64)           ;; 失效守卫：入口+回边比对，不等→返回 STALE(5)

rtcl.unbox_int(h: i32)   -> (i32 ok, i64 v)   ;; multi-value（Wasm 2.0）
rtcl.unbox_float(h: i32) -> (i32 ok, f64 v)
rtcl.box(v: i64) -> i32                       ;; tagged → handle（仅必要时）
rtcl.box_float(v: f64) -> i32
rtcl.binary_op(op: i32, a: i64, b: i64) -> i64 ;; 通用兜底：宿主完整 Tcl 语义
rtcl.get_var(name_h: i32) -> i64
rtcl.set_var(name_h: i32, v: i64)
rtcl.call(cmd_id: i32, argv_base_h: i32, argc: i32) -> i64  ;; 命令调用永远走宿主
rtcl.set_result(v: i64)                       ;; 取代 M0 版，收 tagged word
```

- **错误通道**：哨兵 tagged word（bits==2，即 bit0=0 + 保留 handle 值），
  宿主置 pending error，JIT 每次跨边界调用后一条 `br_if` → return
  TCL_ERROR。不用 wasm 异常（exnref 生态太新），不 trap（`catch` 语义
  要求错误是普通值）。
- **立即数零往返**：`PushInt` 不再需要 `push_int` 调用（M0 的
  `emit.rs:73-76` 模式废弃）。
- **wasm→wasm 直通**：宿主以裸 `extern "C"` 导出（纯数值签名，
  wasm-bindgen 不包 wrapper）直接塞进 JIT 模块 import object → JIT→宿主
  = wasm→wasm 调用，省 JS trampoline 与 i64↔BigInt 转换。接线级优化，
  不改语义，单独可测。
- **jit-native（wasmtime）同一 ABI**，host 侧用 `Caller` 实现；js-string
  相关的 externref 导入仅 `StringMode::JsBuiltin` 出现（§6）。
- **arena 纪律**：跨边界即移交所有权；`run()` 返回后 arena truncate 到
  常量水位线（防 handle 泄漏）。

## 4. Int 快车道 lowering（M1' 核心）

守卫：操作数 `bit0==1` 才内联（两条指令），否则 `binary_op` 宿主兜底
（shimmer "42"→Int、bignum、报错全在宿主）。

| 运算 | 策略 | 细节 |
|---|---|---|
| 比较 `== < <= > >= !=` | 全内联 | tagged word 是 2v+1 形式，i63 域上单调——**免 untag 直接 `i64.lt_s` 等** |
| `+ -` | 内联+检查 | untag → `i64.add/sub`（\|结果\|<2^63 不溢出）→ ~5 条指令范围检查（结果须落回 [-2^62, 2^62)，即 `s>>62 ∈ {0,-1}`）→ 越界回宿主精确 BigInt 宽化 |
| `*` | 内联快速路径 | 检查两操作数高 32 位均为符号扩展（\|a\|,\|b\|<2^31，~4 条指令）→ 积 <2^62 安全；否则回宿主 |
| `/ %` | **全走宿主** | Tcl 是 floor division，wasm 向零取整；除零与 MIN/-1 trap 要拦，内联得不偿失 |
| 位运算 | 内联 | i63 域内 `and/or/xor/shl` 直接做；`shr` = `i64.shr_s`（与 Tcl 算术右移一致） |
| `incr` | 内联 | `IncrLocal` 超指令 = add 特化 |

**slot 单态缓存**（M2）：`LoadLocal` 守卫一次成功 → wasm local 缓存
untagged i64 + dirty 位；后续 `IncrLocal`/算术直接用，写回时 retag。
`for` 循环体编译后 ≈ 原生 wasm 循环 + 每迭代一次守卫——对标 tclsh
5–11× 差距的关键（tclsh 的 expr 也要走 Tcl_Obj 往返，内联后我们更便宜）。

## 5. Float 快车道（开关 `jit-float-lane`，默认 off）

- **表示**：f64 lane 与 i63 lane 并存于 wasm locals。**不做 NaN-boxing**
  （i63 放不进 f64 的 2^53 尾数；边界每次编解码；背离宿主表示）。
- **纪律**：`unbox_float` 守卫成功 → f64 local 缓存 → 算术/比较内联 →
  穿越边界或守卫失败时 `box_float`（走现有 freelist，≈一次内存复用）。
- **混合运算**：int lane untag 后 `f64.convert_i64_s` 提升（Tcl 语义）。
- **语义清单（逐条对齐 tclsh 8.6）**：
  - 类型分派先于 opcode：`expr 1/2==0` vs `expr 1./2==0.5`；
  - **float→string 永远宿主**：shortest-round-trip 格式化 +
    `tcl_precision`，须与 tclsh 逐字节一致，绝不在 JIT 里格式化；
  - **transcendental 永远宿主**（sin/cos/pow/sqrt）：Rust 与 C libm 可能
    有 ULP 差异，单一事实来源留宿主，差分门裁决；
  - **NaN 是合法值**（`expr {0.0/0.0}`），不用 NaN 哨兵——multi-value
    的 ok 标志即为此设立；
  - NaN/Inf 比较 = IEEE754 = C double，可内联；double→int 转换
    （Inf 报错等 Tcl 特有规则）走宿主。
- **开关语义**：off = float 全走 `binary_op`（路线 A 行为）；on = 发射
  守卫+快路径。**正确性不依赖开关，性能才依赖**——与 int lane 同纪律。
- **落地形态**：cargo feature `jit-float-lane` + `EmitConfig.float_lane`；
  三向差分全绿后才评估翻默认。

## 6. Lazy string（开关 `lazy-string`，rtcl-wasm 默认 on，独立于 JIT）

**机制**：JS 起源字符串默认保持 JS 原生形态（externref JS string），首次
需要字节级语义时才物化为 rtcl 原生 UTF-8 表示；反向同样惰性（宿主串首次
穿越边界才建 JS 形态并缓存池索引）。即「跨边界的 shimmer」——把 Tcl
自身的双重表示哲学延伸到 JS 边界。**不依赖 JIT**：解释器主路径（JS→
Tcl→JS 的 echo/转发/UI 事件）今天就受益。

### choke point

所有字节消费收敛于 `Value::as_str`/`as_bytes` 的 OnceCell 物化
（`value.rs:746-779`）——惰性钩子只加在这一个入口，调用方零改动。

### 表示与接线（对齐 `host.rs` trait 层模式）

```rust
enum InternalRep { ..., HostStr(u32) }        // 宿主字符串池索引

trait HostStringBacking {                      // rtcl-vm 不认识 JS
    fn to_bytes(idx: u32) -> SmallVec<u8>;    // 物化 UTF-16→UTF-8
    fn equal(a: u32, b: u32) -> bool;         // 免物化比较
    fn concat(a: u32, b: u32) -> u32;         // 免物化拼接
    fn release(idx: u32);                     // 池槽位回收（挂 freelist drop 路径）
}
```

wasm32：rtcl-wasm 注册 `JsStringBacking`（`Vec<Option<JsString>>` + 空闲栈，
js_sys externref 表保活）。非 JS 宿主无人注册 → variant 编译期不构造、
运行期不出现，零成本。`ValueInner` 大小不变（enum 已被 Vec/DictMap 撑大）。

### 状态机（双向）

```
JS→Tcl：JsOnly ──首次字节操作──→ Dual（JS串 + 物化字节缓存）──mutation──→ NativeOnly
Tcl→JS：NativeOnly ──首次穿越──→ Dual（缓存池索引，二次穿越零成本）
```

### 免物化白名单（逐条验证后生效）

| 操作 | 实现 | 安全性依据 |
|---|---|---|
| `string equal` / `eq` | JS strict equality / builtin `equals` | UTF-8↔UTF-16 对合法标量值是双射 |
| `string compare` | builtin `compare` | UTF-8 字节序与 UTF-16 码元序均保码点序（surrogate 区段的编码设计使然） |
| concat / `string cat` | JS string concat | 合法串「拼接→转码」≡「转码→拼接」 |
| 回传 JS / puts 到 console / UI 事件 | 直接取 externref | 零转码，收益本体 |

### 强制物化清单

list/dict parse（`lindex`/`foreach`/`lappend`/ExpandList）、`string
map/match/range/index/replace/tolower/first/last`、regexp、expr 数值解析、
`binary scan/format`、`eval`/`uplevel`/替换（parser 要字节）。

### `string length` 刻意移出白名单

JS `.length` 数 UTF-16 码元；tclsh 8.6 UCS-2 语义 astral 字符 = 2；rtcl
现状疑似 code point（Rust chars，= Tcl 9 语义）。三者未实测前走物化路径
（冷操作 + 物化后有缓存）。验证手段：`specs/string.allium` 加 astral
probe，judge 差分裁决，别猜。

### lone surrogate 防线

JS string 可含孤立 surrogate，Tcl 串不能。入边界一次校验，含孤立
surrogate 的串立即转码 + 替换符兜底，永不以 HostStr 形态存在——白名单
的安全性论证都建立在「合法标量值」前提上。进测试。

### JIT 模块侧（v2，可延后）

```wat
rtcl.as_externref(h: i32) -> externref    ;; 不物化，直接取 JS 串
rtcl.from_externref(externref) -> i32     ;; 包成 HostStr handle
```

JIT 里的 `Concat`/`Streq` 可编译为 builtin `concat`/`equals`，全程不出
externref。依赖 `StringMode::JsBuiltin`；wasmtime 自动回退 host 调用。
编码只需普通 import + externref global（wasm-encoder 0.261 的
`RefType::EXTERNREF` 够用，已核实）；`importedStringConstants` 让常量
字面量成为零拷贝 import 的 externref global（编译时
`new WebAssembly.Module(bytes, {builtins:['js-string'], importedStringConstants:'_'})`；
js-sys 不收 options，需 `Reflect` 构造或小段 JS shim）。Tcl 字面量可含
任意字节——含高位/非法 UTF-8 的字面量退回 `rtcl.const_str` import。

## 7. 编译器处理架构（主罗盘 = Chez Scheme 及其 Nanopass 后继）

**架构取向裁决**：rtcl-jit 的编译管线不对标 V8 式多级投机框架，而采用
**Chez Scheme 的哲学 + 其后继 Nanopass 架构**（Andy Keep 重写 Chez 后端，
ICFP'13）——「编译快到无需分级」+「几十个小 pass，每个只做一件事」。
理由与落地形态如下。

### 7.1 Chez Scheme 的三条启示

1. **编译足够快，就不需要分级**。Chez 的 REPL 不是解释器：每行输入即时
   完成编译、优化、寄存器分配、**直接生成机器码**（不过汇编器），其
   「编译+执行」快过绝大多数 Scheme 解释器的「解释」，甚至快过不少
   Scheme 编译器优化后的产物。V8 式多层分级存在的理由是编译贵；rtcl
   的既有约束（per-proc 模块 <4KB、同步实例化、预算 <1ms）本身就是
   Chez 式增量编译——所以 **M1' 的目标是「所有可编译 proc 都编译」
   （基线档），M4 的 tier-up 只是锦上添花而非主体**。工业级实证见
   §7.4 的 Racket CS：三种编译模式里「默认全编译 + 解释器仅兜底超大
   模块」转正，而 per-lambda 懒编译 JIT 模式被官方判定「performs
   poorly and probably should be discontinued」。
2. **复杂度放在运行时，不在编译器**。Chez 的性能根基在运行时（数据
   表示、bump 分配、分代 GC），编译器只是「翻译」——把一种语言的
   语义用另一种语言表达。对应 rtcl：值表示（tagged i63 / freelist /
   slot 帧 / lazy-string）才是性能主战场。这正是阶段一（榨干解释器
   架构）与 JIT 设计的一致点，也是本设计把 GC/值生命周期全部留在
   宿主侧（§1-§3）的深层理由。
3. **寄存器分配委托给目标引擎**。Chez 后继版把 regalloc 换成图着色：
   编译慢 2-2.5× 换执行快 17-25%。wasm 目标下这笔账不用付——wasm
   locals 无限，V8/wasmtime 自带高质量 regalloc，rtcl-jit **零成本拿
   到同档收益**，这是 wasm 目标相对 native 的结构性优势。

### 7.2 Nanopass 的工程论据（Chez 后继架构）

- **起源**：印第安纳大学的观察——传统编译器只有 ~5 个大 pass，是因为
  早期内存小、pass 间只能靠文件 IO 传数据；内存不再是瓶颈后，应该拆成
  几十个小 pass：每个只做一件事（函数式拆大函数的同理），改动只需测
  一小块，bug 不连锁。传统大 pass「一动就要全测、常引发连锁反应」正是
  传统编译器复杂度巨高的主因。
- **实测驳回反对意见**：反对者认为 50+ pass = ~10× 语法树遍历 = 无法
  商用。ICFP'13 论文（Keep & Dybvig）的硬数字：旧编译器 **10 个大
  pass**、自编译全部源码 ≈3 秒（2013 硬件）；Nanopass 重写后 **~50
  pass / ~35 个形式化中间语言**，生成代码快 **15-27%**，编译时间控制
  在 ≤2×（最差均值 1.75×，大头来自顺带换上的图着色 regalloc）。
  Chez v10.0 release notes 复现了同一笔账：新后端（nanopass 序列 +
  图着色 RA）编译慢 ≤2×，换生成代码快 **14-40%**。部分收益直接归因于
  Nanopass 的可迭代性（易改、bug 少 → 敢多试优化）。
- **可测试性纪律**：每个 pass 配全套测试、提交即反馈（Dybvig 编译器
  课程的模式）——与本仓库已有文化同构（judge 三向差分 / probe 即时
  对拍 / allium 行为规格）。
- **对 rtcl 尺度的核算**：per-proc op 序列 ~50-200 个，8 个 pass ≈ 千级
  节点遍历，纯内存操作，纳秒-微秒级，远在 <1ms 编译预算之内。「10×
  遍历恐惧」在 Chez 的尺度已被证伪，在我们的尺度不成立。

### 7.3 rtcl-jit 的 Nanopass 化管线（M1' 的 emit.rs 按此拆分）

**IR 层级**（每层形式化定义 + Debug/disassemble + 独立可测）：

| 层 | 定义 |
|---|---|
| L0 | `rtcl_ir::ByteCode`（现有：op 序列 + 常量池 + sites + epoch） |
| L1 | L0 + lane 标注（每 op 操作数假设 Int/Float/Any + 守卫点位） |
| L2 | L1 + slot→wasm-local 分配决策（哪些 slot 进 i64/f64 local，哪些留宿主） |
| L3 | wasm 线性指令序列（结构化指令级 IR，非字节） |
| L4 | wasm 字节模块（wasm-encoder 输出） |

**Pass 表**（每个 = 纯函数、单向变换、可独立 golden 测试）：

| Pass | 变换 | 职责 | 对照经典阶段 |
|---|---|---|---|
| `annotate_lane` | L0→L1 | 常量类型传播 + 可选画像 → lane 假设 | 类型推断 |
| `escape` | L0→L0 注解 | upvar/array/global/别名触及的 slot 标不可缓存 | 逃逸分析 |
| `plan_guards` | L1→L1 | lane 假设处插守卫（类型/epoch/回边 fuel 点） | 投机规划、deopt 点 |
| `alloc_locals` | L1→L2 | slot→wasm local 映射（i64/f64 分道） | regalloc 前置（实际分配委托引擎） |
| `lower` | L2→L3 | op→wasm 指令模板（快车道模板 + 兜底 import 模板） | instruction selection |
| `instrument` | L3→L3 | 回边 fuel 自减、epoch 比对织入 | — |
| `assemble` | L3→L4 | wasm-encoder 模块组装（types/imports/exports/code） | 后端 |

**测试纪律**：每个 pass 配 golden 测试（输入 IR 文本 → 输出 IR 文本）；
端到端正确性由 judge 三向差分兜底。pass 数量允许演进——Nanopass 的
要点不是数量而是「单职责 + IR 形式化 + 独立可测」。Chez 直接生成机器码
不过汇编 ↔ rtcl-jit 用 wasm-encoder 直接构造字节、不过 wat 文本/外部
工具——已对齐。跨 proc 内联明确排除在近期之外（proc 可被
rename/redefine，失效面太大），M5+ 再评估。

### 7.4 Chez 后继项目实证

- **Racket CS（最大成功案例，「一个语言编译到成熟高性能运行时」的
  范本）**：分层 = rumble（Chez Scheme 写的运行时层）→ schemify
  （Racket expander 输出 linklet → Chez Scheme 形式）。schemify 本身
  就是 nanopass 风格实证：~20 个文件 ≈9k 行，全是单一职责小 pass
  （inline/lift/left-to-right/simple/known…）。linklet 直引 ~1500 个
  primitives，移植只给 Chez 打了 ~30 个补丁。Racket 8.0（2021-01）CS
  转正时 1800+ 包仅 12 个编译失败；CS 比 BC 更快且性能更稳定，从源码
  构建省 ~20% 内存。三种编译模式的结局最有信息量：**默认全编译**
  （machine-code）转正、解释器只给超大模块兜底
  （`PLT_CS_COMPILE_LIMIT`=10000）、**per-lambda 按需 JIT 被官方判定
  「performs poorly and probably should be discontinued」**。
  三点启示：①「全编译 + 解释器兜底超大输入」是「快到无需分级」的
  落地形态，支持 rtcl 在 proc 定义点即时编译；②**不要给 rtcl-jit
  设计热点懒编译触发器**（M4 据此重新定位，见 §9）；③性能大头来自
  runtime primitives 的质量而非 codegen 聪明程度——rtcl 应把精力投在
  宿主 Rust helper 与值表示上（§2 原则 3 的又一佐证）。
- **Idris2**：默认代码生成目标就是 Chez Scheme（备选 Racket/Gambit/
  JS/RefC），自举后自编译 ~90 秒（2021）。一个严肃语言可以把「成熟的
  高性能平台」当唯一默认后端——rtcl 选 wasm 为目标、把优化下放给
  引擎，与此同构。
- **Nanopass 是可复制方法论，不是 Chez 私有技巧**：nanopass-framework-
  scheme 独立库仍活跃（2026-09 有 push）；Ghuloum《An Incremental
  Approach to Compiler Construction》（2006，24 步增量造编译器）是
  教程级编译器的源头之一；Siek《Essentials of Compilation》把该方法
  教材化；Indiana 课程十五周造 43-pass 编译器。rtcl 拆 pass 有现成的
  话语体系与测试范式可搬。
- **Wastrel（2026，Andy Wingo，无血缘旁证）**：wasm→C→native 全程序
  AOT，GC 直接取自 Whippet 库——「编译器保持简单、复杂度收进运行时
  库」在 2026 年仍被高手选择（同 benchmark 比浏览器跑同一份 wasm 快
  5-6×，作者自测单 benchmark，弱证据）。同时证明 wasm 作为编译 IR
  的生态（Hoot→wasm→再编译）已经成立。
- **Chez 自身仍活跃**：10.0（2024-02，新增 aarch64/riscv64/loongarch64
  与 pb 可移植字节码后端）→ 10.4.1（2026-05），约半年一版。pb 后端
  印证「一套编译器 + 可移植兜底后端」的分层（对应 rtcl 的解释器兜底
  档位：wasip1/embedded 永不 JIT）。

### 7.5 参考代码索引（Chez `s/` 目录，行数为 2026-10-17 对 main 实测）

先读论文定调（ICFP'13，commercial-nanopass.pdf），官方导读
`IMPLEMENTATION.md` 给地图：cmacros.ss=对象布局；syntax.ss=宏展开；
**cpnanopass.ss + cpprim.ss = 主编译器**；cp0/cptypes/cpletrec = 其前
的 source-to-source pass；x86_64.ss 等 = 后端。

| 文件 | 行数 | 职责 | 优先级 |
|---|---|---|---|
| `s/np-languages.ss` | 1315 | ~35 个 IR 的形式化谱系定义 | ★ 第一个读 |
| `s/compile.ss` | 2351 | 编译驱动：pass 顺序与组装 | ★ |
| `s/cprep.ss` | 334 | 前置整理 | ★ 建立小 pass 手感 |
| `s/cpvalid.ss` | 574 | 合法性检查 | ★ |
| `s/cpletrec.ss` | 420 | letrec 变换 | ★ |
| `s/cpcommonize.ss` | 586 | 公共子表达式 | ★ |
| `s/cp0.ss` | 5880 | source 级部分求值/内联，性能主力 | ★★ |
| `s/cpnanopass.ss` | 10980 | 主下降管线（源 IR → 机器相关 IR） | ★★ |
| `s/cpprim.ss` | 8617 | primitives 内联为未装箱操作 | ★★（类比 rtcl helper 内联决策） |
| `s/cptypes.ss`(+lattice) | 2534+1547 | 类型推断与格 | 选读 |
| `s/x86_64.ss` | 3548 | 后端：instruction selection 即模式匹配 | ★（看一个即可） |
| `s/cpcheck.ss` | 224 | **独立 IR 校验 pass** | ★ 模式照抄 |
| `s/syntax.ss` / 其余后端 / FFI 等 | 大 | 与 codegen 无关或重复 | 跳过 |

**建议阅读顺序**：np-languages.ss（IR 家谱长什么样）→ compile.ss
（pass 怎么串）→ cprep/cpvalid/cpletrec/cpcommonize（四个几百行文件，
建立「单职责小 pass」手感）→ cp0.ss（source-to-source 优化天花板）→
cpnanopass.ss + cpprim.ss（主菜）→ x86_64.ss（确认后端只是模式匹配 +
发射）。**`cpcheck.ss`（224 行）单独注意**：它是独立的 IR 校验 pass，
正是「每个 pass 独立可测」的具体形态——§7.3 每层 IR 配 checker +
golden 测试就是照抄这个模式。

### 7.6 同思路编译器对照（「直译、无 IR、复杂性下沉」一系）

| 编译器 | 做法 | 关键数字 | 对 rtcl-jit 的启示 |
|---|---|---|---|
| Sparkplug（V8，2021） | 从字节码直译（自称 "cheats"），**无 IR，实现 = 「for 循环里一个 switch」**，栈帧与解释器兼容，复杂语义全调 builtins | Speedometer +5–10%，真实基准主线程 −5–15% | 从 rtcl ByteCode 直译 wasm、帧约定与解释器一致、复杂语义下沉 host helper——有工业级背书 |
| Winch（wasmtime 基线编译器） | 单 pass、无 IR、轮转 RA（不够就全 spill）、复用 Cranelift 发射器当汇编库 | NEAR 实测 Cranelift 编译比单 pass 慢 ~20×，因而迁 Winch | rtcl 处境更好：wasm 是栈机，连轮转 RA 都不必做，locals + 操作数栈即可 |
| CPython copy-and-patch（3.13） | 构建期 clang 编每条字节码的 C handler 为 stencil（参数留 0 = hole），运行时拷贝+打洞拼接 | 首批基准 +2–9% | wasm-encoder 直发天然同构（字节模板+填洞）；wasm 模块加载验证器 = 免费的正确性检查（CPython 没有的福利） |
| Lightening（Guile，Wingo） | 因「只想要简单」从 GNU lightning 2.x fork；可移植汇编器、无 IR | Guile 采用后 **JIT 阈值降低 50×**（编译快才敢早编译）；新后端移植 ~20 小时 | wasm-encoder 就是 rtcl 的 Lightening——一个永远不用移植的后端；编译够快 ↔ 无需分级，与 Chez 同一枚硬币 |
| MIR / QBE | 轻量 SSA 后端（vs LLVM） | MIR sieve 编译 75µs vs GCC-O2 13.35ms（≈180×），执行仅慢 6%；只做简单 RA+指令合并 ≈ GCC -O2 的 80%；QBE「70% 性能、10% 代码量」 | **自建优化层的收益上限 ≈ -O2 的 70-80%**，而 wasmtime 已在 wasm 层做 SSA 优化——把编译速度吃满、优化外包给引擎，不自建 MIR |
| Oberon / Go / OCaml | 快编译作为长期工程纪律 | Oberon 编译器 <2900 行、自编译 ~3 秒（HN 转引，弱来源）；Go FAQ：特性会因「影响编译速度或设计清晰度」被拒 | **Wirth 式纪律的 rtcl 版：任何让 per-proc 编译延迟超过 REPL 可感知阈值（~1ms）的优化，默认拒绝** |

### 7.7 MoonBit 编译器管线（wasm 原生多后端对照，源码实证）

MoonBit 编译器 2024-12 开源（[moonbitlang/moonbit-compiler](https://github.com/moonbitlang/moonbit-compiler)，**OCaml 编写**，开源部分含 wasm-gc 后端）。以下 pass 序列为直接读管线驱动源码（`src/driver_util.ml`）所得一手事实——这是除 Chez 外第二个「多小 pass 串联」的工业级实证，且是**唯一以 wasm 为一等目标的现代样本**。

**两段式结构**（官方 LLVM 后端公告首次公开全貌）：

```
moonc build-package（包级增量）：AST → TAST（类型标注）→ Core IR（ANF！不是 SSA）
  → Core_Inline_Single_Use_Join   （单次使用 join point 内联）
  → Eliminate_async               （async 脱糖/状态机化）
  → Core_Contify                  （contification，可选开关）
  → Core_Remove_Let_Alias         （别名消除 ≈ 拷贝传播）
  → Core_Stackalloc               （不逃逸可变记录解盒/栈化）
  → Core_Unbox_Loop_Params        （循环参数解盒）
  → Core_Propagate_Constr         （构造器传播 ≈ 常量传播变体）
  → Core_Lambda_Lift              （lambda lifting）
  → Core_DCE                      （包级死代码消除 DCE = Dead Code
                                   Elimination：以 pub 定义为根砍不可达
                                   代码；全文件仅 182 行）
moonc link-core（链接时全程序）：
  → Core_link → Monofy            （worklist 单态化，以 export 为根；兼作链接时 DCE）
  → Pass_layout.optimize_layout   （数据布局优化）→ MCore
  → Clam_of_core                  （闭包转换 → CLAM 低层 IR）
  → Wasm_of_clam_gc → Shrink_wasmir（自研 dwarfsm 编码器直出 wasm，不经 Binaryen）
```

**wasm-gc 值表示（源码实证，补上次调研缺口）**：带 payload enum =
struct（field 0 = i32 tag，payload 从 index 1 开始）；闭包 = struct
（field 0 = funcref 代码指针，后续为捕获变量），调用走 `struct_get 0` +
`call_ref`；字符串 = UTF-16 array，或开 `use_js_builtin_string` 后走
js-string builtins。若未来评估 §1 的 WasmGC 远期实验，这套编码可直接参照。

**对 rtcl-jit 的十条借鉴**：

1. **两段式粒度**：包级增量 + 链接时以导出为根的激进裁剪 → rtcl 映射为
   **proc 级编译单元缓存 + 以入口 proc/命令为根的按需生成**——亚毫秒
   预算下「只为可达单元生成代码」是最有效的时间与体积控制。
2. **ANF 而非 SSA**：实现成本低的刻意选择——佐证 §7.3 不建 SSA、直接
   在栈式 ByteCode 上加注解 pass（L0-L3）的决定。
3. **DCE 根的分层**：包级保守（pub + trait 方法全留，动态分派不可判定）、
   链接时激进（export 为根）→ Tcl 版：proc 内对动态调用保守，实例化时
   按实际调用图裁剪。
4. **contification 替代 tail-call 提案**：仅尾位置调用的局部函数编译成
   `loop`+跳转，不依赖 wasm tail-call 提案（dwarfsm 指令集里根本没有
   return_call）——Tcl 的 `tailcall` 命令与尾递归 proc 的同思路处理，
   避免 wasm 栈耗尽。
5. **switch 分派启发式（现成规则）**：case ≥3 且值域稠密 → `br_table`
   跳表（带 min 偏移修正）；否则 `i32.eq` 链；字符串 → 顺序
   `string_equal`——直接用于 Tcl `switch` 与命令分派。
6. **小 pass 粒度的又一实证**：`core_dce.ml` 仅 182 行；逃逸分析拆成
   `unbox_mut_records` / `unbox_loop_params` 两个窄 pass。
7. **自研 wasm 编码器直出**（dwarfsm，不经 Binaryen）——与
   wasm-encoder 直发同路线，第三家佐证（前两家：M0 裁决、Lightening）。
8. **编译速度工程**：626 个包 1.06s（同口径 Rust 9.01s、Go 2.56s）；
   函数级并行 + 容错增量分析。编译器本体 OCaml 编写、**以 wasm 形态
   发行**（moonc.js 跑在 Node）——编译器 wasm 化的重度自用样本。
9. **RC 优化 = Perceus 风格（推测，线性内存/C 后端未开源）**：native
   博客的内存管理 benchmark 直接引用 Perceus 论文（borrow 推断消
   incref/decref + refcount==1 原地复用）。rtcl 当前是动态 RC +
   freelist；若未来 JIT 侧要消减 RC 抖动（如 arena 借出值免 bump
   refcount），Perceus 是参考文献。
10. **边界对照**：MoonBit 能「为 DCE 设计语言」（删掉妨碍分析的特性）；
    Tcl 全动态（rename/unknown/upvar）——JIT 须在 proc 边界内定义
    「可静态证明」子集，超出即回退解释执行（与 §7.10 的 tclquadcode
    教训同构）。

### 7.8 其他编译器分级谱系（对照，非罗盘）

| 引擎 | 分级 | 关键机制 |
|---|---|---|
| V8 | Ignition 解释器 → Sparkplug 基线 JIT（2021，无优化模板化直译）→ Maglev（2023，SSA 中端）→ TurboFan（sea-of-nodes 顶层） | feedback vector 采集类型画像；投机优化 + deopt（帧翻译回解释器） |
| HotSpot | 解释器 → C1（带 profiling）→ C2（tiered level 1-4） | OSR：长循环回边中途切换编译版本 |
| JSC | LLInt → Baseline → DFG → FTL | Baseline 同为模板化直译 |
| LuaJIT | tracing：热循环录制 trace，线性 IR | snapshot 记录每点解释器帧状态，guard 失败按 snapshot 重建 |
| Mono jiterpreter | IL trace → wasm 模块 → 实例化，不支持即回解释器 | per-trace 模块粒度；rtcl-jit M0 的直接对标（HANDOFF §6） |
| CPython | 3.11 PEP 659 特化自适应解释器（字节码按 site 画像原地特化，~1.25×）→ 3.13 实验 copy-and-patch JIT（3.14 仍 `PYTHON_JIT=1` 可选，实测 -10%~+20%） | stencil 模板拷贝+常量打洞；特化失败原地反特化 |
| wasmtime | Winch 基线 + Cranelift 优化档 | 基线快编译占位，优化档异步替换 |

### 7.9 可借鉴的六个模式（对照系分级引擎）

1. **分级与画像驱动特化**（V8/HotSpot）：tier-up 靠调用计数 + 类型反馈。
   rtcl 对应物：调用计数阈值机制（M4，已降级为可选的画像增强重编译，
   见 §9）；画像 = vm_exec 侧 per-site 操作数类型计数（轻量、可选），
   JIT 据此选 lane 假设而非纯运行时守卫。节制教训：V8 的 feedback
   vector 是全局基础设施，我们只在编译决策点加计数，不铺满。
2. **基线 JIT 思想**（Sparkplug/Baseline/Winch）：第一档不做优化，只做
   「去掉 dispatch 的模板化直译」，编译极快、立即见效。M1' 的守卫式
   lowering 本质就是这个档位（与 §7.1 的 Chez 启示一致）——先求
   「所有可编译 proc 都编译」，再求优化深度。
3. **deopt/帧重建**（V8 eager/lazy deopt、LuaJIT snapshot）：guard 失败
   必须能回解释器且状态无损。rtcl 的纪律：guard 失败只发生在 op 边界
   （不在表达式中间），回退 = 以当前 arena/栈状态调宿主 import 继续——
   值在宿主 arena、变量在宿主 frame，帧天然共享，比 V8 帧翻译简单一个
   量级。**保持此性质：永不把未物化的 JIT 私有状态带出 guard 点。**
4. **OSR**（HotSpot）：长跑循环在回边 tier-up。rtcl 对应：回边的 fuel
   检查点天然是挂点——回边计数超阈值返回特殊码，宿主编译完重入。
   远期（M5+），且仅当「定义即编译」的基线档仍暴露出 gap 时才评估。
5. **copy-and-patch / stencil**（CPython 3.13+）：代码生成 = 预编译模板
   拷贝 + 常量打洞，编译压到微秒级。rtcl 侧：wasm-encoder 已是纯内存
   数据构造（无 Cranelift 级重后端），per-proc 模块 <4KB；若 M4 后编译
   延迟成瓶颈，可预生成模块模板字节、运行时只 patch 常量段（i64.const
   的 LEB 位置）。列为评估项，不预做。
6. **产物二次优化**（Binaryen wasm-opt）：V8 在 WasmGC 产物上实测
   wasm-opt 平均再提速 ~1.9×。rtcl per-proc 模块太小，多半不值得挂
   Binaryen（且违背 HANDOFF §6 不引 C++ 的裁决）；若未来做 AOT 全脚本
   编译（单模块体量上来），再评估 wasm-opt 作为离线 pass。

### 7.10 反面教材与边界

- **tclquadcode**（Tcl 官方 LLVM JIT，停滞）：全程序 AOT 被语义角落
  （trace/unknown/ensemble）拖死。教训 = 坚持 per-proc 粒度 + 解释器
  永远兜底 + 不能 byte-exact 就整体留解释器。
- **CPython JIT 收益区间**（3.14 实测 -10%~+20%）：字节码→JIT 的收益
  受画像质量限制。M1' 验收（arith_loop ≥3×）以 bench 实测为准，不为
  JIT 而 JIT。
- **wasm 不可自修改代码**：「JIT」在 wasm 内只能是「生成新模块 + 实例
  化」，无 inline patch。CPython 式原地特化不可行——特化决策必须在
  发射前完成（= 发射时快照 + epoch 失效，§3）。
- **per-lambda 懒编译分级的墓志铭**：Racket CS 的 JIT 模式官方结论
  「does not perform well and probably should be discontinued」——当
  基线编译足够快时，懒编译触发器是纯复杂度（§7.4）。rtcl-jit 不设计
  热点触发器作为默认路径。
- **不要自建图 IR / 寄存器分配**：GNU lightning 2.x 从「直接写字节」
  改建图 + RA + DCE 后复杂度失控，Wingo fork 出 Lightening 回归简单
  （§7.6）。wasm-encoder 就是 rtcl 的 Lightening——任何「在 rtcl-jit
  里加一层图 IR」的提议默认拒绝；MIR/QBE 的数字给出自建优化层的收益
  上限（≈ -O2 的 70-80%），而 wasmtime/V8 已在 wasm 层做这些。

## 8. 开关矩阵与验证

| 开关 | 形态 | 默认 | 作用域 |
|---|---|---|---|
| `lazy-string` | cargo feature + interp option | rtcl-wasm **on**，其他平台编译期裁掉 | 值表示层，不依赖 JIT |
| `jit-float-lane` | cargo feature + `EmitConfig.float_lane` | **off**，差分绿后评估 | JIT emitter |
| `StringMode::{HandleOnly, JsBuiltin}` | emitter 参数 | 按宿主特性检测 | JIT emitter |

**差分矩阵**：float_lane × lazy_string 四组合，judge 三向（tclsh /
interp / jit，`judge/run.sh --engine jit`）全绿为门。新增 probe：
`specs/string.allium` 加 astral 字符 length/compare/equal；
`specs/expr.allium` 加 `1/2` vs `1./2`、`0.0/0.0`、Inf 转换、mixed 比较。

**性能假设（先测后承诺）**：

- A：wasm→wasm import ≈ 原生调用 + 边界检查（<5ns），比 JS trampoline
  便宜一个数量级（V8/SM/JSC 三引擎实测，SM/JSC 是风险点）。
- B：Int 快车道让 expr 密集 proc 反超 tclsh（bench/cases 三向）。
- C：per-proc 同步编译 <1ms；超预算 → 先解释执行 + 后台
  `WebAssembly.compile` 异步替换。
- D：`importedStringConstants` 对 per-proc 小模块未必净收益——测完再定
  是否做 §6 的字面量切口。
- E：lazy-string 物化命中率（Dual 占比）——高说明负载字节密集，惰性
  无害但收益有限；JS↔Tcl echo 路径转码次数应从 2 降到 0。

## 9. 里程碑映射（修订 HANDOFF §6）

| 里程碑 | 内容 | 变化 |
|---|---|---|
| M0 | handle-arena 三函数 ABI | ✅ 已完成（`0b5177a`） |
| M1' | ABI v2 整体切换 + Int 快车道 + fuel/epoch globals + emit.rs 按 §7.3 拆分为 nanopass 链 | 修订原 M1；步数预算缺口（TODO）在此补齐 |
| M2 | proc body + 原语；slot 单态 int 缓存 | 不变 |
| M3 | epoch 接 Tier1 bump 信号 | M1' 已埋线，此处接线 |
| M4 | **重新定位**：默认 proc 定义即编译（Chez/Racket CS 模式，§7.4）；热点计数仅驱动可选的画像增强重编译，不作默认路径 | 修订：原「~50 阈值 tier-up」降级——per-lambda 懒编译已被 Racket CS 证伪 |
| C-lazy | lazy-string（§6） | 新增；**独立于 JIT，可先行** |
| C-float | jit-float-lane（§5） | 新增；依赖 ABI v2，默认 off |
| C-str | js-string-builtins 两切口（§6） | 新增可选；依赖 StringMode 检测 |

依赖关系：C-lazy ⊥ JIT；C-float ⊂ ABI v2；M1' 不依赖任何 C-* 项。
每里程碑 gate 不变：judge 三向全绿 + bench 不回退 + 单测绿。

## 10. 风险清单

1. SM/JSC 的 wasm→wasm import 性能未验证 → 假设 A bench 前置。
2. js-string builtins 非 JS 宿主不存在 → StringMode 开关是硬要求。
3. arena 生命周期：跨边界即移交所有权，`run()` 后 truncate 到常量水位线。
4. lone surrogate / 非 UTF-8 字面量边界过滤（§6），漏了出静默语义偏差。
5. ABI v2 是 breaking change：M0 三函数废弃，wasmi roundtrip 测试与
   `engine.rs` import 文档同步更新。
6. `string length` 三方语义分歧未裁决（§6）——先物化，后验证。
7. Wasmtime GC 刚默认开（2026-07）且性能尚不如 V8——WasmGC 远期实验
   的触发条件之一。

## 11. 来源

- 标准与支持：[Wasm 3.0 Completed](https://webassembly.org/news/2025-09-17-wasm-3.0/) · [Chrome WasmGC 默认启用](https://developer.chrome.com/blog/wasmgc) · [Wasmtime GC & Exceptions](https://bytecodealliance.org/articles/wasmtime-gc) · [webassembly.org/features](https://webassembly.org/features/)
- 移植指南：[V8: A new way to bring GC languages to Wasm](https://v8.dev/blog/wasm-gc-porting)
- MoonBit：[FFI/ABI 文档（双后端 + RC 约定）](https://docs.moonbitlang.com/en/latest/language/ffi.html) · [JS String Builtins 博客](https://www.moonbitlang.com/blog/js-string-builtins)
- Go/TinyGo：[TinyGo GC 原理](https://aykevl.nl/2020/09/gc-tinygo/) · [golang/go#63904（不上 WasmGC 的原因）](https://github.com/golang/go/issues/63904)
- 各语言案例：[dart.dev/web/wasm](https://dart.dev/web/wasm) · [Kotlin 2.3.20 js-string builtins](https://kotlinlang.org/docs/whatsnew2320.html) · [Wasocaml 论文](https://inria.hal.science/hal-04311345/document) · [Wingo: WebAssembly for the rest of us](https://www.wingolog.org/archives/2023/03/20/a-world-to-win-webassembly-for-the-rest-of-us)
- RC 案例：[Pyodide 类型转换/内存](https://pyodide.org/en/stable/usage/type-conversions.html) · [PEP 818](https://peps.python.org/pep-0818/) · [MicroPythonOS wasm 端口记录](https://docs.micropythonos.com/web-port/developer/)
- 编译器架构：[CPython copy-and-patch JIT（3.13 起）](https://schoolofweb.net/en/posts/python-jit-compiler-copy-and-patch/) · [Faster CPython/PEP 659 收益](https://www.odbms.org/2025/12/on-the-evolution-of-python-qa-with-diego-russo/) · [Python 3.14 性能总览](https://blog.csdn.net/u010339879/article/details/163296803)
- Chez/Nanopass：[guenchi：Chez Scheme 与 Nanopass 的故事](https://guenchi.com/0x7c06.html) · [王垠：我不是编译器专家](https://github.com/AlexiaChen/YinWangBak/blob/master/%E6%88%91%E4%B8%8D%E6%98%AF%E7%BC%96%E8%AF%91%E5%99%A8%E4%B8%93%E5%AE%B6.md) · [Chez Scheme 编译指南（含 rationale）](https://www.cnblogs.com/lsgxeva/p/16586940.html) · [Dybvig ICFP'06 架构 talk](http://icfp06.cs.uchicago.edu/dybvig-talk.pdf) · [Dybvig: The Development of Chez Scheme](https://www.cs.indiana.edu/~dyb/pubs/hocs.pdf) · [Keep & Dybvig ICFP'13（commercial nanopass 论文）](https://www.cs.indiana.edu/~dyb/pubs/commercial-nanopass.pdf) · [Chez v10.0 release notes](https://cisco.github.io/ChezScheme/release_notes/v10.0/release_notes.html) · [IMPLEMENTATION.md（源码导读）](https://github.com/cisco/ChezScheme/blob/main/IMPLEMENTATION.md)
- 后继项目：[Racket CS README（三种编译模式）](https://github.com/racket/racket/blob/master/racket/src/cs/README.txt) · [ICFP'19 A Racketeer in Chez's Clothing](https://www-old.cs.utah.edu/plt/publications/icfp19-fddkmstz.pdf) · [Racket 8.0 status 博客](https://blog.racket-lang.org/2021/01/racket-status.html) · [schemify 源码目录](https://github.com/racket/racket/tree/master/racket/src/schemify) · [Idris2 后端文档](https://idris2.readthedocs.io/en/latest/backends/index.html) · [nanopass-framework-scheme](https://github.com/nanopass/nanopass-framework-scheme) · [Ghuloum: An Incremental Approach to Compiler Construction](http://scheme2006.cs.uchicago.edu/11-ghuloum.pdf) · [Wastrel 里程碑博文](https://wingolog.org/archives/2026/04/09/wastrel-milestone-full-hoot-support-with-generational-gc-as-a-treat)
- 同思路编译器：[V8 Sparkplug](https://v8.dev/blog/sparkplug) · [Wasmtime Winch README](https://github.com/bytecodealliance/wasmtime/blob/main/winch/README.md) · [NEAR 迁 Winch（Cranelift 编译慢 20× 实测）](https://blog.nearone.org/research/2026/07/01/replacing-nears-contract-vm-with-wasmtime.html) · [copy-and-patch 论文](https://fredrikbk.com/publications/copy-and-patch.pdf) · [Wingo: Lightening](https://wingolog.org/archives/2019/05/24/lightening-run-time-code-generation) · [MIR 项目](https://developers.redhat.com/blog/2020/01/20/mir-a-lightweight-jit-compiler-project) · [QBE](https://c9x.me/compile/) · [Go FAQ（编译速度是设计约束）](https://go.dev/doc/faq)
- MoonBit 编译器：[moonbit-compiler 开源仓库（OCaml，含 wasm-gc 后端）](https://github.com/moonbitlang/moonbit-compiler) · [开源公告](https://www.moonbitlang.cn/blog/compiler-opensource) · [LLVM 后端公告（管线全貌）](https://www.moonbitlang.com/blog/llvm-backend) · [首发布公告（626 包 1.06s、为 DCE 设计语言）](https://www.moonbitlang.com/blog/first-announce) · [native 后端（Perceus 风格 RC）](https://www.moonbitlang.com/blog/native) · [wasm.io 2025 演讲 slides](https://2025.wasm.io/slides/moonbit-and-webassembly-unlocking-the-true-efficiency-of-wasm-wasmio25.pdf) · [Perceus: Garbage Free Reference Counting with Reuse](https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/)
