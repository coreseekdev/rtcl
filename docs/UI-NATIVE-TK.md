# rtcl 原生 UI 工具包（现代 Tk）— 方向与架构

状态：方向决策（2026-11-08）。与 `docs/UI-DSL.md`（`ui::*`，web page /
WebView 内的 AI 声明式 UI）**分轨**：本文的 `tk` 轨道是原生应用开发，
目标是替代标准 Tcl/Tk 的 Tk 控件层，不是同一套东西。

## 1. 目标与非目标

**目标**

- 原生 iOS / Android / 桌面（macOS / Windows / Linux）控件，而非 WebView。
- Tcl/Tk 编程模型：widget 路径、`button .b -text ...`、`pack`/`grid`、
  `bind`/`after`/`update`/`vwait`、`-textvariable` 等尽量兼容。
- 控件本身现代化：原生文本/输入/滚动/虚拟列表、主题、深色模式、
  无障碍；后续接入 table/log/tree/chart/editor 等现代组件。
- 单进程嵌入 rtcl-vm，Tcl 脚本可独立打包或嵌入宿主 App。

**非目标（v1）**

- 不追求 Tk 的像素级/全部选项兼容（`-relief/-bd/-highlightthickness`
  等 90 年代视觉选项按主题白名单裁剪）。
- 不复用 `ui::*` catalog/op 协议；那套继续只服务 web/WebView。
- 不承诺 Canvas/Tk 8.6 全量绘图语义；Canvas 作为后续独立组件。
- 不在首版做拖拽设计器；catalog 驱动的检视器后置。

## 2. 基础选型：nsh 作为原生渲染层

| 候选 | 移动端原生 | 桌面 | Rust 可嵌入 | 结论 |
|---|---|---|---|---|
| **nsh 0.4** | ✅ UIKit / Android views | ✅ AppKit/Win32/GTK4/GPUI/TUI | ✅ `nsh-core`/`nsh-base`/platform crates（Apache-2.0） | **采用** |
| gpui-kit | ❌ | ✅ | ✅ | 仅桌面后端，后置可选 |
| mygo | WebView 为主 | ✅ | ❌（Go） | 仅作 WebView/打包参考，不采用 |

nsh 的 Rust 组件 API（`nsh-core::prelude`、`nsh-base`）提供 view/text/
input/button/nav/data_table/editor 等原生组件，并有 `each`/`when`/signals
动态组合；正好承载“Tcl 生成/修改 widget 树”的模型。iOS/Android 通过
nsh 的 `NshAppView`/`NshView` 嵌入宿主。

## 3. 总体架构

```
┌──────────────────────────── 应用进程（原生） ────────────────────────────┐
│  Tcl 脚本（package require Tk）                                         │
│      │  tk button/label/entry/frame、pack/grid、bind、after、vwait      │
│      ▼                                                                  │
│  tk 兼容层（Tcl + Rust 命令）                                            │
│      │  WidgetSpec 树 / 选项校验 / 布局 / 变量绑定 / 事件脚本            │
│      ▼                                                                  │
│  tk-core（后端无关）                                                     │
│      │  shadow widget tree + 事件队列 + 脏标记 + diff                   │
│      ▼                                                                  │
│  nsh backend（Rust 组件）                                                │
│      │  nsh-core signals / each / when；nsh-base 组件                  │
│      ▼                                                                  │
│  nsh-platform-{ios,android,macos,windows,linux,gpui}                    │
│      └─ UIKit / Android views / AppKit / Win32 / GTK4 / GPUI            │
└─────────────────────────────────────────────────────────────────────────┘
```

**线程模型（建议 v1）**：Tcl 解释器与 nsh UI 在**同一主线程**，事件循环
合一——这正是 Tk 的语义（widget 命令同步、`update` 可重入、`vwait`
等待事件）。长任务会阻塞 UI，与 Tk 一致；后台能力后续用 `thread`/
`after` 扩展，不改变 Tk 单线程默认。`ui::*`（AI/Web）仍按 Worker 模型
独立演进。

**为什么不是独立 Tcl 线程**：Tk 兼容 API 大量同步查询（`winfo`、`cget`、
`update`）要求命令返回即状态可见；跨线程往返会把每个 widget 命令变成
同步 RPC，容易死锁且与 Tk 语义不符。单线程是兼容性优先的正确默认；
需要并发时由显式后台机制承担。

## 4. Tk 兼容层分级

### L0（v1 必须）

- 解释器启动：`package require Tk`、`tk::init`、根窗口 `.`、
  `wm title/geometry/resizable/withdraw/deiconify`。
- 基础控件：`frame`、`label`、`button`、`entry`、`checkbutton`、
  `radiobutton`、`scale`、`text`（只读/简单编辑）、`toplevel`。
- 布局：`pack -side/-fill/-expand/-padx/-pady`、`grid -row/-column/
  -rowspan/-columnspan/-sticky/-padx/-pady`、`place` 基础子集。
- 选项：`-text/-textvariable/-command/-variable/-value/-state/-width/
  -height/-font/-foreground/-background/-padx/-pady/-anchor/-justify`。
- 事件：`bind`（键/鼠标/`<<Virtual>>`）、`event generate`、`after`、
  `update`、`vwait`、`focus`、`destroy`。
- 查询/配置：`cget`、`configure`、`winfo children/width/height/exists`、
  `winfo screenwidth/screenheight`。
- 错误：Tk 回调错误 → `bgerror`；Tcl 错误信息保留路径/命令上下文。

### L1（常用应用）

- `ttk::*`：`treeview`、`notebook`、`progressbar`、`combobox`、
  `scrollbar`、`separator`。
- `listbox`、`menu`、`scrollbar`、`panedwindow`、`labelframe`。
- `-image`/图标、字体/主题映射、DPI、深色模式、IME、无障碍。
- 现代组件：虚拟化 table、log、tree、chart、code editor（nsh-base 已有）。

### L2（后置/明确差异）

- Canvas 绘图、Tk 8.6 全部 photo/image、复杂 text tag、菜单栏/托盘。
- 非白名单 Tk 选项：接受但告警/忽略，或直接报“not supported in modern Tk”。

## 5. Crate 布局（建议）

```
crates/rtcl-tk-core/     # WidgetSpec、选项 schema、布局、事件、shadow tree
crates/rtcl-tk-runtime/  # rtcl-vm 嵌入 + tk 命令注册 + 事件循环桥
crates/rtcl-tk-nsh/      # nsh-core/base 组件映射 + 平台封装
crates/rtcl-tk-testing/  # headless 驱动（nsh-testing / 纯模型测试）
apps/rtcl-tk-demo/       # 原生 demo（Rust main + 示例 .tcl）
```

`rtcl-tk-core` 不依赖 nsh，可在 CI/无 GUI 环境用纯模型测试；nsh 只出现在
后端 crate。原生平台编译矩阵：

- Linux 开发需 GTK4 dev（当前环境未安装）；GPU/GPUI 后端可作 CI 桌面路径。
- iOS 需 macOS/Xcode；Android 需 NDK；二者走 nsh 的 embed 工程模板。
- 无 GUI CI：`nsh-testing` headless + tk-core 模型测试。

## 6. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M0 | tk-core WidgetSpec/选项/布局/事件模型 + 纯模型单测 | 无 GUI 下 `button .b ...; pack .b` 生成正确树 |
| M1 | rtcl-vm 嵌入 + `tk` L0 命令 + nsh 后端最小集（frame/label/button/entry） | Linux/GPUI 或 headless demo：按钮点击执行 Tcl 命令 |
| M2 | pack/grid、bind/after/update/vwait、textvariable、cget/configure/winfo | 经典 Tk 小脚本可直接跑（差异表外） |
| M3 | ttk 常用控件 + listbox/menu/scrollbar + 主题/DPI/IME/a11y | 设置页/表单/列表演示 |
| M4 | 虚拟化 table/log/tree/chart + iOS/Android embed + 打包 | 手机/桌面同一 Tcl 脚本跑通 |

## 7. 主要风险

1. **nsh API 演进**：锁定 0.4.x，封装在 `rtcl-tk-nsh`，不让 tk-core 泄漏 nsh 类型。
2. **桌面系统依赖**：GTK4/WebKit dev 包不在当前环境；CI 需预装或走 GPUI/headless。
3. **Tk 选项/布局语义差距**：pack 顺序敏感 vs flex 模型、grid 轨道/权重映射，需要差异表与回归脚本。
4. **事件循环**：`vwait`/`update` 与 nsh 调度器整合是最大技术风险，M1 必须先验证。
5. **移动端嵌入**：iOS/Android 工程、生命周期、键盘/安全区、签名打包。
6. **Canvas/text**：复杂控件后置，避免首版范围失控。

## 8. 首个 spike（建议）

在无 GTK4 的环境先做 **headless + GPUI** 两条中最容易的一条：

1. `rtcl-tk-core`：WidgetSpec 树 + `button/label/frame/entry` + pack 子集 + 单测。
2. 一个 nsh Rust demo（`nsh-platform-gpui` 或 `nsh-testing`）渲染该树；
   Tcl 脚本 `button .b -text Hi -command {puts clicked}; pack .b` 点击后
   回调进入 Tcl。
3. 通过后再接 iOS/Android 平台 crate 与真实设备。

在此之前不把 nsh 加入 workspace 依赖，先在独立 spike crate 验证 API/构建。
