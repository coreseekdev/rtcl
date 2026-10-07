# rtcl — 浏览器可嵌入的 Tcl 解释器（Rust）

轻量、跨平台的 Tcl 兼容解释器，主目标为 **浏览器 WASM VM**，同时覆盖
原生嵌入式场景。与 tclsh 8.6.17 逐字节对齐（4067/4072 官方测试用例），
性能关键路径已达到或超过 tclsh。生产就绪状态见
[docs/PRODUCTION.md](docs/PRODUCTION.md)。

## 特性

- **Tcl 8.6 兼容**：37 个语料文件 + 4072 个官方测试提取用例逐字节判定；
  语义全集含 namespace、TclOO、dict、expr 宽化、channel 等
- **双引擎**：字节码 VM（主）+ 树遍历（语义回退），`judge/sweep.sh`
  双引擎差分 0 分歧
- **WASM 主目标**：wasm-bindgen 绑定（exec / JS 命令注册 / puts 路由 /
  实例隔离），release 1.6MB，node 测试 18/18
- **性能**：算术/自增/字符串/字典微基准反超 tclsh（0.6-0.8×）；
  过程调用开销反超（0.52×）；重型基准 1.05-2.26×（详见
  bench/BASELINE.md）
- **36 项扩展**：`lambda`/`curry`/`function`/`loop`/`ensemble`/`defer`/
  `json`/`ref`/`flow` 等（经裁定的超出标准 Tcl 部分）

## 快速开始

### 原生构建与运行

```bash
cargo build --release
./target/release/rtcl -f script.tcl   # 执行脚本（注意 -f）
./target/release/rtcl -c "puts hello" # 内联求值
./target/release/rtcl -i              # REPL
```

### 浏览器 WASM

```bash
cd crates/rtcl-wasm
./build.sh            # wasm-bindgen web 目标 -> pkg/
node test.js          # node 测试套件（18 项断言）
```

```js
import { RtclHandle } from "./pkg/rtcl_wasm.js";
const tcl = new RtclHandle();
tcl.set_output_handler((s) => console.log(s));
tcl.register_command("js_alert", (msg) => alert(msg));
const result = tcl.exec('puts "hello from Tcl"; js_alert "hi"');
```

### Rust 宿主嵌入

```rust
use rtcl_core::{Interp, Value};

let mut interp = Interp::new();
interp.eval("set x 42").unwrap();
```

## 命令覆盖

核心命令全集：`set`/`puts`/`if`/`while`/`for`/`foreach`/`lmap`/`switch`/
`expr`（含 bignum 宽化）/`proc`/`apply`/`lambda`/`namespace`（完整）/`dict`/
`string`（全子命令）/`list`（全子命令）/`binary`/`chan`/`file`/`exec`/
`clock`/`regexp`/`regsub`/`info`/`uplevel`/`upvar`/`trace`/`catch`/`try`/
`error`/`throw`/`array`/`scan`/`format`/`subst`/`split`/`join`/
`oo::class`/`oo::object`/`oo::define`/`oo::copy`/`self`/`my`/`next` +
36 项扩展。逐命令语义由判官语料钉死。

## 表达式

标准 Tcl 表达式语义：i64 + 自动 bignum 宽化、浮点 Inf/NaN、
字符串比较回退、`abs`/`int`/`double`/`round`/`floor`/`ceil`/`sqrt`/
`pow`/`sin`/`cos`/`tan`/`log`/`exp`/`min`/`max` 等。

## 项目结构

```
rtcl/
├── crates/
│   ├── rtcl-core/     # 解释器核心（命令全集、双引擎、判官钉死的语义）
│   ├── rtcl-ir/       # 字节码指令集 + ByteCode
│   ├── rtcl-parser/   # 解析器 + 字节码编译器（含 peephole 超指令）
│   ├── rtcl-vm/       # Value 表示 + 遗留执行器
│   ├── rtcl-jit/      # JIT 原型（暂停；解释器为主要引擎）
│   ├── rtcl-wasm/     # wasm-bindgen 绑定（浏览器主目标）
│   ├── rtcl-cli/      # 命令行接口
│   └── rtcl-expect/   # expect 风格进程自动化
├── judge/             # 判官：tclsh 官方测试提取语料 + 双引擎差分
│   ├── run.sh         #   87 文件 4067/4072 用例逐字节判定
│   ├── sweep.sh       #   双引擎差分（0 分歧门槛）
│   └── extract/       #   tclsh 测试套件 -> 语料的机械提取器
├── bench/             # 性能台账 BASELINE.md + 基准脚本
└── docs/PRODUCTION.md # 生产就绪状态
```

## 质量门槛（提交纪律）

```bash
cargo test --workspace                      # 1143 测试
./judge/run.sh                              # 87/87 文件逐字节
./judge/sweep.sh                            # 双引擎差分 0 分歧
cargo check -p rtcl-core --target wasm32-unknown-unknown --no-default-features --features std
```

四门全绿才可提交；判官/差分必须用刚构建的二进制（防陈旧二进制假绿）。

## 与标准 Tcl 的差异

5/4072 用例存在已知分歧（0.12%，全部记录于 docs/PRODUCTION.md）：
rename 遮蔽过程后旧名解析、`interp alias {} name {} target` 形式、
3 条错误信息帧计数微差。另有 36 项经裁定的扩展超出标准 Tcl。

## 平台支持

| 目标 | 状态 |
|---|---|
| 原生（Linux/macOS/Windows） | ✅ |
| wasm32-unknown-unknown（主目标） | ✅ 1.6MB + node 套件 18/18 |
| wasm32-wasip1 | ✅ |
| no_std + alloc（embedded 特性） | ⚠️ 已知破损（记录在案） |

## 许可

BSD 2-Clause（同 jimtcl）
