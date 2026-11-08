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

## 5. Crate 布局与 feature 门禁

```
crates/rtcl-tk-core/     # WidgetSpec、选项 schema、布局、事件、shadow tree
crates/rtcl-tk-runtime/  # rtcl-vm 嵌入 + tk 命令注册 + 事件循环桥
crates/rtcl-tk-nsh/      # nsh-core/base 组件映射 + 平台封装（M1）
crates/rtcl-tk-testing/  # headless 驱动（nsh-testing / 纯模型测试）
apps/rtcl-tk-demo/       # 原生 demo（Rust main + 示例 .tcl）
```

**可选 feature（与 wasm 互斥）**

- `rtcl-cli` 的 `tk` 是 **opt-in feature**，默认关闭；开启才拉
  `rtcl-tk-runtime` 与后续 nsh 后端。
- `rtcl-tk-runtime` 是 **native-only** crate：`target_arch = "wasm32"`
  下直接 `compile_error!`，不提供 wasm 构建。
- `rtcl-wasm`/`ui::*` 永不依赖 Tk 轨道；二者不能在同一构建里混用。
- 原生宿主也可直接依赖 `rtcl-tk-runtime`，不必经过 CLI feature。

`rtcl-tk-core` 不依赖 nsh，可在 CI/无 GUI 环境用纯模型测试；nsh 只出现在
后端 crate。原生平台编译矩阵：

- Linux 开发/CI 需 GTK4 dev（本机已装 `libgtk-4-dev` 4.22；
  `nsh-platform-linux` 0.4 + 最小 GTK4 app 已构建并启动验证）。
- iOS 需 macOS/Xcode；Android 需 NDK；二者走 nsh 的 embed 工程模板。
- 无 GUI CI：`nsh-testing` headless + tk-core 模型测试。

## 6. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M0 ✅ | tk-core WidgetSpec/选项/布局/事件模型 + 纯模型单测；tk-runtime native-only 骨架 + `tk` feature 门禁；nsh GTK4 最小 app 构建/启动 | 无 GUI 下 `button .b ...; pack .b` 生成正确树；`cargo build -p rtcl-cli --features tk` 通过；wasm 目标对 tk-runtime 报 compile_error |
| M1a ✅ | rtcl-vm `tk` 命令层：frame/label/button/entry/… 创建、configure/cget、pack/grid、destroy、winfo、wm/tk；per-widget 路径命令；teardown 自动清树；4 条单测 | `button .b -text Hi; cget .b -text` 等 Tcl 脚本语义通过 |
| M1b ✅ | nsh 后端最小集（frame/label/button/entry）+ TkView；按钮 `-command` 执行 Tcl，按钮文本经 signal 刷新；GTK4 demo + headless 验收 | `nsh-testing`：初始 "Click me" 渲染 → press → "Clicked 1" → press → "Clicked 2"；`cargo run -p rtcl-tk-demo` 打开 GTK4 窗口 |
| M2 ✅ | pack/grid/place、bind/event generate、after/update、`-textvariable` 双向、entry/text/listbox/scale 数据子命令、winfo/wm 扩展、ttk 别名、focus/raise/font/option；message/scrollbar/separator/progressbar；整树 dynamic 重建 | 经典 grid/pack 表单脚本通过（`classic_nsh`）；demo 双态截图；tk 单测 19 + headless 验收 2 |
| M3 ◐ | ttk notebook/combobox/treeview/labelframe、entry 校验、虚拟化列表/树、Tk scrollbar 联动、主题色、macOS 系统菜单、Canvas 动态/绑定/滚动/旋转/平滑/图片、photo image、text tag 富文本+编辑 ✅；Windows/Linux 原生菜单受 nsh GTK 限制 | 设置页/表单/列表演示（classic demo + headless 验收） |
| M4 | 虚拟化 table/log/tree/chart + iOS/Android embed + 打包 | 手机/桌面同一 Tcl 脚本跑通 |

## 7. 主要风险

1. **nsh API 演进**：锁定 0.4.x，封装在 `rtcl-tk-nsh`，不让 tk-core 泄漏 nsh 类型。
2. **桌面系统依赖**：本机 GTK4 已就绪；CI 需预装 `libgtk-4-dev`（WebKit 仅 WebView 特性需要）。
3. **Tk 选项/布局语义差距**：pack 顺序敏感 vs flex 模型、grid 轨道/权重映射，需要差异表与回归脚本。
4. **事件循环**：`vwait`/`update` 与 nsh 调度器整合是最大技术风险，M1 必须先验证。
5. **移动端嵌入**：iOS/Android 工程、生命周期、键盘/安全区、签名打包。
6. **Canvas/text**：复杂控件后置，避免首版范围失控。

## 8. 首个 spike（建议）

M0 已完成：`rtcl-tk-core` 纯模型 + `rtcl-tk-runtime` native-only 骨架 +
`rtcl-cli` 的 `tk` feature 门禁；nsh 0.4 `nsh-platform-linux` 最小 GTK4
app 已在本机构建并启动。

M1a/M1b/M2 已完成：

- `rtcl-tk-runtime`：Tk 命令层 + shadow tree + active interpreter；
  `-textvariable`/`-variable` 读写、数据子命令、bind/event、布局查询。
- `rtcl-tk-nsh`：`TkView` 用 `dynamic` 区域订阅 revision，任何 Tcl
  变更都整树重建（新增/删除 widget 同样生效）；16 ms 泵 `after`。
- `apps/rtcl-tk-demo`：GTK4 runner + headless 验收
  （`tk_nsh`、`classic_nsh`）+ `shot` 截图 bin。

## 9. 已实现命令面与差异（M2/M3）

**控件**：`toplevel/frame/label/button/entry/checkbutton/radiobutton/
scale/text/listbox/message/scrollbar` + `ttk::{separator,progressbar,
notebook,combobox,treeview,labelframe,button,label,entry,frame,checkbutton,
radiobutton,scale,text,listbox,scrollbar,message,canvas}` 别名；扩展
`::rtcl::chart`（原生柱状图，`-values/-labels/-title/-width/-height`）。

**命令**：`configure/cget/cget 前缀缩写/destroy/pack/grid/place/winfo/
wm/tk/bind/bindtags/event generate/focus/raise/lower/font/option/bell/
tk_focusNext/tk_focusPrev/menu`；控件子命令含 `invoke/get/insert/delete/
index/icursor/selection/size/curselection/step` 等；菜单子命令含
`add/insert/delete/index/entrycget/entryconfigure/invoke/type/post/
unpost`；`ttk::notebook` 的 `add/insert/forget/tabs/select/index/tab`；
`ttk::combobox` 的 `get/set/current/values/post/unpost`；
`ttk::treeview` 的 `insert/delete/item/selection/focus/children/parent/
index/heading/column/exists`；listbox/treeview 按 `-height` 放进滚动视图（滚轮可滚）；Tcl 侧的 `after/update/vwait` 沿用 rtcl-vm。

**菜单**：`menu` 窗口与 `-menu` 关联的菜单栏。macOS 通过 nsh
`desktop::set_application_menu` 映射到系统菜单；Linux/Windows 在 app 内
渲染为一行 cascade 按钮 + 浮层下拉（command/checkbutton/radiobutton/
separator/cascade），headless 同样用 app 内菜单以便测试；`-tearoff`
接受但不绘制撕下菜单。

**布局**：pack 按子控件顺序流式排列，连续 `-side left/right` 归入同一
行（右侧项推到行尾），不放 fill 的 parcel 从行/列首开始；grid 按行分组、`columnconfigure -weight` 进入
flex 权重；place 支持绝对坐标与 `-relwidth/-relheight`，相对坐标按父
容器比例解释。混合 side 的精确 Tk 收缩顺序是近似。

**窗口**：`wm title` 在桌面后端实时写入原生窗口标题
（`nsh desktop::set_title`）；其余 `wm geometry/state` 仅记录在模型。
`tk scaling`/`tk appname` 可读写；`winfo fpixels/pixels` 按 scaling 换算
（i/m/c/p 单位），`winfo screenmmwidth/screenmmheight` 有值。UI 的实际
DPI 缩放由 nsh 平台层负责。

**字体**：`-font` 支持命名字体（`font create`）与 Tk 描述串
（`{DejaVu Sans} 12 bold`）；label/button/check/radio/message 应用
family/size/bold（实际 DPI 缩放仍由 nsh 平台层负责）。

**Canvas/图片/文本标签**：`canvas` 支持 `rectangle/oval/line/text/
polygon/image` 的 `create/delete/coords/itemconfigure/itemcget/find/
move/scale/raise/lower/bind/xview/yview`；`-scrollregion` + xview/yview
平移裁剪；item 或 tag 的 `<Button-1>` 绑定按命中项执行（近似 hit test）；
`-angle` 旋转、line `-smooth` 二次曲线、image 项绘制图片。`image create
photo` + `label/button -image` 渲染文件图片。`text tag add/remove/
configure/cget/ranges/names/delete` 维护区间；有标签的 text 先渲染富文本
（颜色/背景/字体跨度），点 Edit 进入纯文本编辑、Done 回富文本，编辑时
标签区间按增删自动平移。

**虚拟化/滚动**：listbox 与 treeview 只挂载可见行（按 `-height` 取窗口），
`yview`/`-yscrollcommand` 与 Tk `scrollbar` 双向联动；列表/树表可滚动。
`text` 原生输入自带滚动。table 用 treeview columns 表达，log 用高行数
listbox 表达。

**主题**：显式 `-background/-foreground`（`#rgb/#rrggbb` 与常用色名）
应用到 label/button/frame/labelframe/message/listbox/treeview；
`-selectbackground/-selectforeground` 控制列表选中色。未显式设置时用
nsh 的自适应深色/浅色配色；DPI/IME/无障碍角色由 nsh 平台层提供。

**Entry 校验**：`-validate key|all` + `-validatecommand`，支持 `%P/%s/
%S/%v/%V/%W` 替换；返回 0 时拒绝编辑并回滚显示（错误则按 Tk 语义放行）。

**事件**：按钮 `<Button-1>/<ButtonRelease-1>`、entry `<Return>/<Key>`、
`bind all`、类名/`bind .` 标签绑定（按 `bindtags` 顺序：widget、class、
toplevel、all）、`event generate`；`%W/%x/%y/%K/%A` 替换；`after` 由
`TkView` 的 16 ms interval 泵入，错误进 `take_errors()`。

**已知差异**：`vwait` 仍会阻塞引擎线程（Tcl 语义如此；原生 UI 会暂停
绘制，长等待请用 `after` 回调）；未实现多 toplevel 窗口、Canvas 的旋转/平滑曲线/图片项已支持（见上）；
带 text tag 的文本编辑态为纯文本（非 WYSIWYG）；Windows/Linux 菜单栏仍是 app 内实现
（macOS 用系统菜单；nsh GTK 的 set_application_menu 实测不显示）；
scrollbar 已与 listbox/treeview 联动。

**视觉验收（不用 TSX）**：Tcl 与 TSX 在同一生态位，因此截图走 Rust/GTK，
不引入 `*.test.tsx` 或 nsh CLI：

```sh
cargo run -p rtcl-tk-demo                       # GTK4 窗口
cargo run -p rtcl-tk-demo --bin shot -- /tmp/tk.png
cargo run -p rtcl-tk-demo --bin shot -- /tmp/tk-classic.png --classic
```

`shot` 用 `gtk::WidgetPaintable` + `gtk::Snapshot` + GSK `render_texture`
离屏渲染初始状态；交互态由 headless 测试覆盖（本机 GTK/Wayland 下，
控件在首次绘制后再更新会快照为空，所以脚本应直接以目标状态启动）。
无显示环境用 `xvfb-run -a`；`RTCL_SHOT_SIZE=WxH` 可换窗口大小。

**测试入口**：

```sh
cargo test -p rtcl-tk-core -p rtcl-tk-runtime -p rtcl-tk-nsh -p rtcl-tk-demo
```

下一步（M3 收尾）：原生
系统菜单栏、滚动条联动、主题与 DPI、IME/无障碍细化；随后 iOS/Android
platform crates 与真机。
