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
- **nsh fork**：`Cargo.toml` 的 `[patch.crates-io]` 把
  `nsh-core`/`nsh-base`/`nsh-engine`/`nsh-testing` 指到
  `coreseekdev/nsh` 的 `rtcl-tk` 分支（基于 v0.4.0，ssh git 源）；补丁：
  `SliderEvent::PressStart`（Tk scale 的 `<Button-1>`）、
  Button/Checkbox/Radio/Slider 的 `Layout(Bounds)` 事件（组件根节点的
  `<Configure>`）、`EventKind::Wheel`（非 scroll 元素的 wheel，
  nsh-platform-linux 无 Scroll 祖先时投递给指针下监听者、有 Scroll 时
  也同时派发监听者，nsh-testing 提供 headless `wheel()`）、
  `RichEditorEvent::Wheel`（富文本编辑器内容区）、
  `MouseEnter/MouseLeave/MouseMove` 事件（Button/Checkbox/Radio/Slider
  组件，nsh-core 补了 `MouseEvent` 的 JsValue）、
  `RichEditorEvent::Motion`（富文本内容区 hover/motion）、
  `RichEditorEvent::Press`/`Hover`/`Release`（内容区按下的文档位置、hover
  的文档位置、抬起，供 text `tag bind` 命中）、
  `nsh-platform-linux::clipboard_text_blocking`（GTK 异步读 + 嵌套主循环
  转同步，供 Tk `clipboard get`）。

## 6. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| M0 ✅ | tk-core WidgetSpec/选项/布局/事件模型 + 纯模型单测；tk-runtime native-only 骨架 + `tk` feature 门禁；nsh GTK4 最小 app 构建/启动 | 无 GUI 下 `button .b ...; pack .b` 生成正确树；`cargo build -p rtcl-cli --features tk` 通过；wasm 目标对 tk-runtime 报 compile_error |
| M1a ✅ | rtcl-vm `tk` 命令层：frame/label/button/entry/… 创建、configure/cget、pack/grid、destroy、winfo、wm/tk；per-widget 路径命令；teardown 自动清树；4 条单测 | `button .b -text Hi; cget .b -text` 等 Tcl 脚本语义通过 |
| M1b ✅ | nsh 后端最小集（frame/label/button/entry）+ TkView；按钮 `-command` 执行 Tcl，按钮文本经 signal 刷新；GTK4 demo + headless 验收 | `nsh-testing`：初始 "Click me" 渲染 → press → "Clicked 1" → press → "Clicked 2"；`cargo run -p rtcl-tk-demo` 打开 GTK4 窗口 |
| M2 ✅ | pack/grid/place、bind/event generate、after/update、`-textvariable` 双向、entry/text/listbox/scale 数据子命令、winfo/wm 扩展、ttk 别名、focus/raise/font/option；message/scrollbar/separator/progressbar；整树 dynamic 重建 | 经典 grid/pack 表单脚本通过（`classic_nsh`）；demo 双态截图；tk 单测 19 + headless 验收 2 |
| M3 ◐ | ttk notebook/combobox/treeview/labelframe、entry 校验、虚拟化列表/树、Tk scrollbar 联动、主题色、macOS 系统菜单、Canvas 动态/绑定/滚动/旋转/贝塞尔/箭头/虚线/图片/polygon/按 tag 删查/closest/overlapping/bbox/hidden、photo image、text tag 光标级 WYSIWYG + marks 数据持久化 + text marks/index 语法/tag 区间平移、菜单 `-accelerator` ✅；Linux 原生菜单（GTK PopoverMenuBar，fork 已支持）、Windows 原生 Win32 菜单栏（真实窗口宿主开启） | 设置页/表单/列表演示（classic demo + headless 验收） |
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

**控件**：`toplevel/frame/label/button/menubutton/entry/spinbox/
checkbutton/radiobutton/scale/text/listbox/message/scrollbar` + `ttk::{separator,
progressbar,notebook,combobox,treeview,labelframe,button,menubutton,label,
entry,spinbox,frame,checkbutton,radiobutton,scale,text,listbox,scrollbar,
message,canvas,sizegrip,panedwindow}` 别名（ttk
创建的控件带 Tk 的 T* 类名，如 `ttk::button` → `TButton`，因此
`bindtags`/`bind TButton`/`winfo class` 与 tkinter 一致）；扩展
`::rtcl::chart`（原生柱状图，`-values/-labels/-title/-width/-height`）。

**命令**：`configure/cget/cget 前缀缩写/destroy/pack/grid/place/winfo/
wm/tk/bind/bindtags/event generate/focus/raise/lower/font/option/bell/
tk_focusNext/tk_focusPrev/menu/clipboard/selection/tk_messageBox/
tk_getOpenFile/tk_getSaveFile/tk_chooseDirectory/tk_chooseColor/tk_optionMenu/tk_dialog/grab/ttk::style/tk_popup`；控件子命令含 `invoke/get/insert/delete/
index/icursor/selection/size/curselection/step` 等；菜单子命令含
`add/insert/delete/index/entrycget/entryconfigure/invoke/type/post/
unpost`；`ttk::notebook` 的 `add/insert/forget/tabs/select/index/tab`；
`ttk::combobox` 的 `get/set/current/values/post/unpost`；
`ttk::treeview` 的 `insert/delete/item/selection/focus/children/parent/
index/heading/column/exists`；listbox 的 `nearest y`/`bbox index` 按
渲染行高 18px 的模型近似（bbox 仅返回可见行，格式 `x y w h`）；
滚动查询（listbox/treeview yview、text/entry/listbox xview）返回 Tk 的
浮点样式（未滚动为 `0.0 1.0`，滚动条联动断言已同步）；
listbox/treeview 按 `-height` 放进滚动视图（滚轮可滚）；Tcl 侧的 `after/update/vwait` 沿用 rtcl-vm。

**弹出菜单**：`tk_popup menu x y ?entry?` 与 `menu post x y` 在指定点弹出
菜单（nsh menu 组件以 1×1 不可见 trigger 锚定，受控打开；选项触发
command、checkbutton/radiobutton 更新变量并执行、外部点击/Escape 关闭并
清状态；`?entry?` 忽略）。不带坐标的 `menu post` 仍是菜单栏 cascade 下拉；
`menu unpost` 同时收起两种。

**ttk::style**：`configure/lookup/map/layout/element create|names/theme
names|use|settings` 维护样式/主题数据库；`theme use` 无参数返回当前主题
（默认 default）。ttk 控件的 `-background/-foreground/-font` 未显式设置时
回退到其 ttk 类名（如 `TButton`）的 `configure` 值；`map` 的状态映射会保存
但不参与状态求值。`grab set ?-global? window/release/current/status` 维护
Tk 抓取模型（不强制平台指针抓取）。

**menubutton**：`menubutton`/`ttk::menubutton`（类名 `Menubutton`/
`TMenubutton`），`-text/-textvariable/-menu/-state/-direction/-indicatoron`；
点击后用 nsh menu 组件弹出 `-menu`（自动定位、外部点击/Escape 关闭），
command/checkbutton/radiobutton/separator/cascade 菜单项可选中并触发；
`tk_optionMenu w varName firstValue ?value ...?` 创建 `.w.menu` 单选菜单
并返回菜单路径（`.w` 为 menubutton，textvariable 为 varName）。

**sizegrip**：`ttk::sizegrip`（`TSizegrip`）渲染右下角三角握把，无交互
（窗口缩放由窗口管理器负责）。

**panedwindow**：`panedwindow`/`ttk::panedwindow`（类名 `Panedwindow`/
`TPanedwindow`），`add/insert/forget/panes/pane/paneconfigure/panecget/
sashpos/sash/identify/state/instate`；`-orient` 决定 nsh
`h_resizable/v_resizable` 分组，分栏用原生可拖动 sash。拖动/`sashpos i
new` 记录真实 pane 尺寸（`on_resize` 持久化），`sashpos i` 返回真实位置
（pane 尺寸和 + i×sashwidth；未布局前只有 -width 求和）；`-weight`
分布：`on_resize` 报告的尺寸按 weight 重新分配并持久化（Tk 的“额外空间
按权重分配”近似）；`-hide` 的 pane 不渲染、`-stretch never` 固定面板；
`-minsize/-width` 映射 panel min_size/size；classic 的
`-sashwidth/-sashrelief` 与 ttk 的 style 选项都接受。

**spinbox**：`spinbox`/`ttk::spinbox`（类名 `Spinbox`/`TSpinbox`），
子命令 `get/set/insert/delete/index/icursor/selection/validate/xview`；
`-from/-to/-increment` 数值步进（`-wrap` 环绕、`-format` 经 Tcl
`format`）、`-values` 列表步进、`-textvariable` 双向、`-command` 在
步进改变后执行；渲染为输入框 + 上下按钮，Up/Down 键按 Tk Spinbox 类行为
步进。初始值取 `-textvariable`，否则 `-values` 首项，否则 `-from`。

**菜单**：`menu` 窗口与 `-menu` 关联的菜单栏。macOS 通过 nsh
`desktop::set_application_menu` 映射到系统菜单；Linux/Windows 在 app 内
渲染为一行 cascade 按钮 + 浮层下拉（command/checkbutton/radiobutton/
separator/cascade），headless 同样用 app 内菜单以便测试；`-tearoff`
接受但不绘制撕下菜单。`-accelerator` 按 Tk 语义**只负责显示**：macOS/Linux
菜单项写入原生快捷键（`Ctrl/Cmd`→mod、`Alt`→alt、`Shift`→shift、F1–F24
与常用命名键），app 内下拉在标签右侧追加同样文本；不实现全局热键绑定
（与 Tk 一致，需用户自行 `bind`）。

**布局**：pack 按子控件顺序流式排列，连续 `-side left/right` 归入同一
行（右侧项推到行尾），不放 fill 的 parcel 从行/列首开始；grid 按行分组、`columnconfigure -weight` 进入
flex 权重；place 支持绝对坐标与 `-relwidth/-relheight`，相对坐标按父
容器比例解释。button 的 `pack -fill x/-both` 与 grid `-sticky ew` 会
横向拉伸到 parcel/cell（其余控件/方向仍按原生近似）。混合 side 的精确
Tk 收缩顺序是近似。

**窗口**：`wm title` 在桌面后端实时写入原生窗口标题
（`nsh desktop::set_title`）；其余 `wm geometry/state` 仅记录在模型。
`tk scaling`/`tk appname` 可读写；`winfo fpixels/pixels` 按 scaling 换算
（i/m/c/p 单位），`winfo screenmmwidth/screenmmheight` 有值。UI 的实际
DPI 缩放由 nsh 平台层负责。

**字体**：`-font` 支持命名字体（`font create`）与 Tk 描述串
（`{DejaVu Sans} 12 bold`）；label/button/check/radio/message 应用
family/size/bold（实际 DPI 缩放仍由 nsh 平台层负责）。

**Canvas/图片/文本标签**：`canvas` 支持 `rectangle/oval/arc/line/text/
polygon/image` 的 `create/delete/coords/itemconfigure/itemcget/find/
move/scale/raise/lower/bind/xview/yview`；arc 的 `-start/-extent`
（度，逆时针）与 `-style arc|pieslice|chord` 映射到椭圆弧路径；`-scrollregion` + xview/yview
平移裁剪；item 或 tag 的 `<Button-1>` 绑定按命中项执行（近似 hit test）；
`-angle` 旋转、line `-smooth true|bezier`（二次/三次贝塞尔）、`-dash`/
`-dashoffset`（逐 item 重置，不会泄漏到后续 item）、`-arrow first|last|
both`、polygon `-fill/-outline`（闭合填充 + 描边）、image 项绘制图片。`delete` 与 `find` 接受 id、`all` 或
空格分隔的 `-tags`（`find withtag/withtags`），即按 tag 批量删/查；
`find closest x y ?halo?`、`find overlapping x1 y1 x2 y2`、`bbox
?tagOrId ...?`（id/tag 的并集，按 Tk 向外取整）；`move/scale/coords/
itemconfigure/itemcget/raise/lower` 均接受 tagOrId（tag 命中全部项）；
text/image 默认 `-anchor center`，渲染与 bbox 都按 `-anchor` 摆放。
`-state hidden` 的项不绘制、不参与 bbox/closest/overlapping/点击，但仍
被 `find` 列出。bbox 对 text 用字数近似（runtime 无字体度量），line
箭头未计入。
`image create
photo` + `label/button -image` 渲染文件图片。`text tag add/remove/
configure/cget/ranges/names/delete/nextrange/prevrange` 维护与遍历区间；有标签的 text 用 nsh
`RichEditor`（ProseMirror starter schema + textStyle）渲染为**光标级
WYSIWYG**：原生光标/选区/IME，标签映射为 color/backgroundColor/
fontSize/fontFamily/bold 标记；编辑后的富文档（ProseMirror JSON）作为
**数据**保存在 runtime：`text getdoc` 导出、`text setdoc` 导入 ProseMirror
JSON，可跨应用重启持久化。正文写回 Tcl 模型、标签区间按增删平移。Tk tag 是 Tcl 侧的样式投影/入口：初始文档由 tags 构建；编辑器每次变更
把 marks 投影回 tags——样式 tag 区间按文档重算，能匹配的已有 tag（如
`hot`）复用，新样式生成 `@rich-…` tag，非样式 tag 保留。因此
`tag ranges/names` 能看到编辑器新格式；外部 tag 变化不覆盖 marks，外部
正文改写按差分平移 marks。`text insert index chars ?tagList ...?` 支持
逐对文本/标签并在插入后把 tag 列表加到新区间，`text replace index1
index2 chars ?tagList ...?` 复用删除+插入路径；Tcl 侧 insert/delete/
replace 会平移已有的 tag 区间（相邻/重叠区间合并）与 mark，insert 光标
按 gravity 随编辑移动，与编辑器路径一致。`text
index` 支持 `insert/end/line.col/line.end`、mark 名、`+/- N chars|
indices|lines` 链与 `linestart/lineend/wordstart/wordend` 限定词；
`mark set/unset/names/gravity/next/previous` 维护标记（`insert` 即光标，
`current` 随编辑平移，gravity 控制插入点的贴边方向）；`edit
modified ?0|1?`/`edit reset` 维护未保存标记。`text -undo 1` 打开撤销：`edit
undo/redo/canundo/canredo/separator` 按 Tk 语义工作，快照保存内容、光标、
tag 区间、mark 与富文档；`-autoseparators`（默认开）时 500ms 内的连续编辑
自动合组，`edit separator` 强制分组，`-autoseparators 0` 让每次编辑独立；
`-maxundo` 限制栈深（0 为不限，与 Tk 一致）；`edit reset` 同时清空历史。
Tcl 侧 `tag add/remove` 这类纯格式命令不单独入栈，但会随下一次文本编辑的
快照一起回滚。富编辑器内部的 Ctrl+Z 仍由编辑器自身的历史处理。
**隐式行尾换行已按 Tk 建模**：`end` 是虚拟内容（正文+行尾 `\n`）末尾
（`index end` 对 "abc" 为 `2.0`）；`count 1.0 end` 含隐式换行（"abc" 为
4），`get 1.0 end` 剥离它（`abc`）；隐式换行不可删除（`delete end-1c`
静默忽略）、不可被 `search` 匹配；`dump 1.0 end` 会输出它
（`text {
} 1.3`）；`insert end` 插在它之前；用户自行插入的行尾换行是
真实内容（`get 1.0 end` 保留）。索引整体在虚拟内容上运算（`L.C`、
`end±Nc`、word/line 修饰符、越界钳制到 end）。
`text see index` 滚动到该字符所在行：富文本经 fork 的
`RichEditorApi::scroll_to`，普通 text 经 input 的 ElementRef `ScrollTo`
命令滚到 `(line-1)×18px`（均不移动 caret）；`text dlineinfo index` 按 18px 行高模型返回
`x y w h baseline`（x=3，w 为 -width 字符宽，行不可见/超出行数/end 位置
返回空串）；`text count ?-chars|-indices|-lines|-update|-elide? index1 index2` 按选项
顺序返回计数（默认 chars，`-lines` 为跨过的显示行数）；`text dump ?-all|-text|-tag|-mark|-image|
-window? index1 ?index2?` 按位置输出 Tk 风格事件流：`tagon/tagoff <tag>
<index>`、`mark <name> <index>`、`text <chars> <index>`、`image <name>
<index>`，text 在 tag 边界、mark 与行首处分段；`text image
create/cget/configure/delete/names` 按 Tk 模型实现：嵌入图片占一个
U+FFFC 字符位（索引随编辑平移、删除占位符即删除图片、undo 可回滚、
`get`/`dump -text` 可见该字符），位置由内部 mark 携带（不进 `mark
names`）；dump -image 输出 `image <name> <index>`。**行内绘制已接入
WYSIWYG**：渲染器把 doc 里的 U+FFFC 段拼接成 ProseMirror image 节点
（attrs: src=photo 文件、width/height=image_dimensions），读回时 image
节点经 leaf_text 映射回 U+FFFC（内容长度不漂移）；merge/dump/see 的
位置映射同样感知 image 节点。**photo 固有尺寸（Tk 语义）**：
`image create photo` 现在从 PNG/GIF/BMP/JPEG 头读取数据自身尺寸
（`image width/height`、`img width/height`、canvas image bbox、text
内联图都按它报尺寸；`-width/-height` 非零时按 Tk 语义裁剪报告值，
`cget -width/-height` 仍返回配置值）；`-data` 接受 Tk 的 base64
payload（嗅探 mime 转成 `data:` URI 交给渲染器）、原始二进制或现成
data URI（Tcl 脚本里 data URI 的 `;` 必须用花括号引用）；最后一个
`-file`/`-data` 生效；文件缺失/无法识别按 Tk 报错且 configure 失败
不改变图片。**Android 图片不绘制的根因修正**：此前插桩发现
`Images.java fetch()` 从未被调用、误诊为 fork 平台层缺口——实际是
rtcl 把 photo 报成 0×0（无 -width/-height 时），Android 的 ImageBox
布局帧为 0、`loadImage` 直接返回，加载器根本没被请求（fork 的
Mounter→Images.request→fetch 管线本就完整，file:// 与 data: 均支持）；
固有尺寸修复后已在模拟器复验：`label -image` 与 text 内联 image
**均绘制**（x86_64 模拟器截图，2026-11-11）。桌面 GTK 快照仍受首帧
局限（nsh image 异步解码后快照为空），交互运行时应可见。
**photo 像素引擎（2026-11-11）**：`put`（行×颜色，`#rgb/#rrggbb/
#rrggbbaa` 或 `{r g b a}` 列表；`-to` 区域内平铺、自动扩图、straight
alpha 合成）、`get x y ?-withalpha?`（越界按 Tk 报错）、`blank`、
`transparency get/set`、`copy src ?-from/-to/-zoom/-subsample/-shrink?`
（先 subsample 后 zoom）、`data ?-from/-grayscale?`（默认颜色行列表，
`-format png` 返回 base64——Tk 返回二进制，这里取文本形态以便
`configure -data` 回灌）、`write file ?-format png/-from/-grayscale?`。
runtime 自带无压缩 PNG 编码器（stored deflate + CRC32/Adler32，无新
依赖）与对应解码器（只解自己写出的形态），像素数组即显示真相：有像素
时渲染源是 `data:image/png;base64`，`data/write → -data/-file` 往返
保留 `get/put` 能力；`-file/-data` 创建的照片头解析可读尺寸但**不解码
像素**（像素操作从透明起算，与 Tk 的差异点）。`read` 仍为 no-op
（需 GIF/PPM 解码器）。
`tag bind tagName ?sequence? ?script?` 把事件绑定到 tag 区间：富文本编辑器
按下时按指针下的文档位置映射成字符下标，命中区间的 tag 依次执行最具体的
sequence（`<Button-1>`、Double/Triple/Quadruple、`<ButtonRelease-1>`、
`<Enter>`、`<Leave>`、`<Motion>`），`break` 停止后续 tag；键盘事件按
`insert` 光标所在区间派发。`<Leave>` 覆盖 tag 之间的移动，指针移出编辑器
内容区时由 fork 的 `RichEditorEvent::HoverOut` 对当前 tag 补发（headless
的 `hover` 帮助函数不模拟跨控件 Leave，测试以运行时单测覆盖）。
`<ButtonRelease-1>` 与其他控件一致在按下派发后立即执行（后端没有独立的
指针抬起钩子）。
`text search ?-forward|-backward? ?-exact|-regexp? ?-nocase? ?-all?
?-count varName? pattern index ?stopIndex?` 支持环绕搜索与 regexp
（Rust regex 语法；Tcl ARE 的 \m/\M 等未覆盖），`stopIndex` 只限制
匹配起点。

**虚拟化/滚动**：listbox 与 treeview 只挂载可见行（按 `-height` 取窗口），
listbox `itemconfigure/itemcget` 支持按行的 `-foreground/-background/
-selectforeground/-selectbackground`，插入/删除会同步平移行选项与选中
索引；
`yview`/`-yscrollcommand` 与 Tk `scrollbar` 双向联动；列表/树表可滚动。
`text` 原生输入自带滚动。`see <index/item>` 让 listbox/treeview 以最小
滚动量把目标行带入窗口。treeview 行标签：`tag configure/cget/names/
has/delete` + `insert/item -tags`，`-foreground/-background` 覆盖整行
颜色（选中色优先）；`column` 读写 `-width/-minwidth/-stretch/-anchor/
-id`（默认 width 200），`-width` 同时作用于表头与单元格；`-columns`
之外的 `-values` 不显示。table 用 treeview columns 表达，log 用高行数
listbox 表达。

**主题**：显式 `-background/-foreground`（`#rgb/#rrggbb` 与常用色名）
应用到 label/button/frame/labelframe/message/listbox/treeview；
`-selectbackground/-selectforeground` 控制列表选中色。未显式设置时用
nsh 的自适应深色/浅色配色；DPI/IME/无障碍角色由 nsh 平台层提供。

**Entry 校验**：`-validate key|all` + `-validatecommand`，支持 `%P/%s/
%S/%v/%V/%W` 替换；返回 0 时拒绝编辑并回滚显示（错误则按 Tk 语义放行）。

**状态**：`-state disabled` 的 button/check/radio 不响应点击、`invoke`
为空操作；菜单项 disabled 不 invoke、系统菜单置灰；entry 的
disabled/readonly 与 `-show` 密码已由原生输入呈现。

**事件**：序列按 Tk 规则匹配——kind + 修饰键子集 + detail
（`<Key>`/`<Key-a>`/`<Control-a>`/`<Button-1>`/`<Control-Button-1>`/
`<Double-Button-1>`/`<Triple-Button-1>`/`<Quadruple-Button-1>`/
`<FocusIn>`/`<FocusOut>`），同一 tag 取最具体的一条，
`bindtags` 顺序 widget→class→toplevel→all；`event generate` 可带
`-x/-y/-key/-button/-state`。点击派发 `<Button-1>`（第二次点击同时派发
`<Double-Button-1>`）与 `<ButtonRelease-1>`，覆盖按钮、listbox/treeview
行、canvas 项/画布，以及 label/frame/labelframe/message/progressbar/
separator/chart 的空白区域、notebook 标签、combobox 箭头、treeview 表头；
`break`（tkinter 的 `return "break"`）会终止后续 bindtags；`focus`
切换派发 `<FocusOut>/<FocusIn>`；entry 与富文本 text 也派发
`<Button-1>`/`<FocusIn>`（点击时同步 Tk focus），富文本按键先走该
text 的 `<Key>`/`<KeyRelease>` 绑定（`break` 阻止编辑器插入），粘贴走
`<<Paste>>`；菜单窗口（`.menu`）也可 `bind`，点击/`.m invoke` 时派发。
`bind all`、类名/`bind .` 均可用。按钮默认类绑定 `<Key-space>` invoke
（check/radio 另有 `<Key-Return>`），可由 `bind` 覆盖；listbox/treeview
点击获得焦点，方向键/Home/End/Prior/Next 移动选择。指针/触摸事件：
`<Motion>`/`<B1-Motion>`（canvas 项也支持）、`<Enter>/<Leave>`、
`<Touch>` 与 `<<TouchStart/Move/End>>`、`<<LongPress>>`、`<<Pan>>`、
`<<Pinch>>`、`<<Rotation>>`；触摸轻点仍合成 `<Button-1>` 路径。
`<Enter>/<Leave>/<Motion>` 挂在各控件根节点（label/frame/labelframe/
message/progressbar/canvas/chart/scrollbar/listbox/treeview/notebook/
combobox/entry/text，以及 button/checkbutton/radiobutton/scale 的组件
根节点）；headless 的 Enter 绑定若触发重渲染，紧随其后的首个 Motion
可能落在旧节点，真实指针连续移动不受影响。
`<Configure>` 在 frame/toplevel/label/message/progressbar/canvas/chart/
entry/listbox/treeview/notebook/combobox/scrollbar 以及
button/checkbutton/radiobutton/scale 的**组件自身节点**布局变化时派发
（`%w/%h` 为新尺寸，fork 的 `Layout(Bounds)` 事件）；scale 的按下派发
`<Button-1>`（fork 的 `SliderEvent::PressStart`）；菜单下拉在打开时
按其面板尺寸派发 `<Configure>`（`bind .file <Configure>`）。
滚轮：listbox/treeview 用原生 scroll view 取得 wheel/pan，派发
`<MouseWheel>`、`<Button-4|5>`（横向 6/7，Shift 作用于
`<Shift-MouseWheel>`），无类绑定时按行高滚动虚拟化切片（原生偏移每次
事件复位，只挂可见行）；canvas/entry/text（含 RichEditor 富文本）通过
fork 的 `EventKind::Wheel` 收 `<MouseWheel>` 与 `<Button-4|5>`（无
Scroll 祖先时平台投递给指针下控件，有 Scroll 时滚动与绑定同时发生）。替换 `%W/%x/%y/%X/%Y/%b/%s/%K/%A/%D/%f/%w/%h/%k/%N/%R/%t/%#/%E/%d/%m`
（缺省用 Tk 的 `??`；`%D` 为滚轮 ±120、pan 位移、pinch×100、
rotation×10）；`event generate` 接受 `-x/-y/-rootx/-rooty/-key/-keysym/
-keycode/-button/-state/-delta/-width/-height/-time/-serial/-sendevent/
-detail/-mode/-focus/-count`；`event add/delete/info` 管理虚拟事件，
物理事件会自动触发映射到它的 `<<Name>>` 绑定；内置虚拟事件
`<<Invoke>>`（按钮/check/radio 默认 `invoke`）、`<<ListboxSelect>>`、
`<<TreeviewSelect>>`、`<<TreeviewOpen>>/<<TreeviewClose>>`、
`<<ComboboxSelected>>`、`<<NotebookTabChanged>>` 在对应状态变化时自动
派发；`pack/grid/place` 首次映射派发 `<Map>`，`forget/remove` 与
`destroy` 前派发 `<Unmap>`；`destroy` 按子→父派发 `<Destroy>`；entry
收到粘贴时派发 `<<Paste>>`（原生输入负责实际插入），scrollbar 箭头
点击派发 `<Button-1>`；点击链支持 Double/Triple/Quadruple。`after` 由
`TkView` 的
16 ms interval 泵入，错误进 `take_errors()`。

**已知差异**：`vwait` 仍会阻塞引擎线程（Tcl 语义如此；原生 UI 会暂停
绘制，长等待请用 `after` 回调）；**多 toplevel**：nsh 只提供一个平台窗口，
因此二级 `toplevel` 以主窗口内的浮层卡片呈现（标题栏+关闭按钮、
`wm title/-geometry/-width/-height`、`withdraw/iconify/deiconify`、
关闭按钮走 `WM_DELETE_WINDOW` 协议否则 destroy；浮层可叠加，默认级联
定位）；它们不是独立 OS 窗口，不能单独最小化（标题栏可拖动：
runtime `move_window` 改写 `-geometry`，拖拽状态按窗口路径存 thread-local，
跨重渲染存活）；
`wm overrideredirect/transient/group` 仅保存不生效；Canvas 已支持
旋转/贝塞尔/箭头/虚线/图片/arc/polygon/锚点与 bbox/find 查询；
text tag 已是光标级 WYSIWYG，marks 以富文档数据持久化（getdoc/setdoc
可跨重启），并镜像回 Tk tag（`@rich-…`）；外部正文改写按差分平移 marks；
`tk_messageBox` 由宿主回调提供原生模态框：Linux 后端注册 GTK
`AlertDialog`（Tcl 命令在嵌套主循环里阻塞到用户选择，`-type` 六种按钮组、
`-default/-cancel` 映射默认/Escape，返回 ok/cancel/yes/no/retry/abort/ignore；
GTK 无窗口标题，`-title` 折进正文首行；`-parent` 命名的 Tk 窗口里只有
"." 是原生窗口，对话框固定挂主窗口——本模型下唯一可能的父窗口）；无后端或
GTK 初始化失败时立即返回默认按钮，便于 headless/非 Linux；`tk_dialog
window title text bitmap default ?button ...?` 用同一 GTK AlertDialog
基础返回按钮下标（按钮数量自定义，关闭对话框返回 default 下标；window
参数只作 parent 名记录，不创建可见 Toplevel）；
`tk_getOpenFile/tk_getSaveFile/tk_chooseDirectory` 同样走宿主回调（Linux
GTK `FileDialog`：`-initialdir/-initialfile/-filetypes` 转 native filter、
`-multiple` 返回 Tcl 列表、保存时补 `-defaultextension`、`-confirmoverwrite`
由 GTK 处理；`-parent` 忽略，`-typevariable` 写第一个 filetype 描述；
无后端时返回空串/空列表表示取消）；`tk_chooseColor` 走宿主回调
（Linux GTK `ColorDialog`，`-initialcolor` 解析 Tk 色名/#hex，返回
`#rrggbb`，取消返回空串；无后端时返回 `-initialcolor`）；`clipboard
clear/append/get` 维护应用自己的文本并同步到系统剪贴板；应用未拥有剪贴板
时 `get` 通过后端的同步 getter 读取系统剪贴板（Linux：GTK 异步读包在嵌套
主循环里，1s 超时；其它平台未接 getter 时返回空）；`selection
own/clear/get/handle` 维护 PRIMARY/CLIPBOARD 所有权，拥有时 `get` 调用本地
handler（`handlerCommand offset maxChars`），未拥有时回退到剪贴板文本，
`-type TARGETS` 返回 `STRING UTF8_STRING`；系统 PRIMARY 选区桥不可行
（GTK4 不暴露 X11/Wayland 的 primary selection，gdk::Clipboard 只有默认
选区），因此非本应用拥有的 PRIMARY 读取走剪贴板；
`<Configure>` 尚未覆盖菜单窗口；`<<Copy>>/<<Cut>>/<<Paste>>/<<SelectAll>>/<<Undo>>/<<Redo>>`
已按 Tk init.tcl 预定义（`event info` 可见，物理 Ctrl+键自动触发其绑定）；
text/富文本路径经修饰键触发，plain entry/Spinbox 由 ambient
`ctrl+c/x/v/a/z/y` 键（nsh 视为非编辑键，输入框聚焦时也触发）投递到
聚焦控件的 `<<事件>>` 绑定；
entry 选区拖动（`<B1-Motion>`）在桌面鼠标拖动时由平台 MouseMove 派发；
headless `drag` 桌面环境下同样派发带左键的 MouseMove（fork 8e7c075），
测试已覆盖；菜单项/separator 等装饰控件的
hover 未单独接（菜单面板本身有 `<Enter>/<Leave>`）；
菜单栏：macOS 系统菜单；Linux GTK app 原生 PopoverMenuBar（fork 平台层
本就支持，rtcl 侧此前只在 macOS 同步是误判，已修）；**Windows 原生
Win32 菜单栏**（fork 的 `SetMenu` 路径，CreateMenu/AppendMenuW 真系统
菜单 + 加速键）：真实窗口宿主经 `set_native_menus(true)` 打开（headless
测试保持 app 内菜单以便点击验证），桌面 runner（apps/rtcl-tk-app）已接；
未打开时 Windows/Linux 维持 app 内实现；scrollbar 已与 listbox/treeview 联动。

**字体度量（2026-11-11）**：runtime 增加 `set_text_measurer` 宿主钩子
（`fn(family, point size, bold, text) -> TextMetrics`），Linux 后端注册
pango 实现（pangocairo FontMap；Tk 正 -size 是磅、负 -size 是像素）。
接入了 `font measure`（含 `-displayof`）、`font metrics`（含无参全列表
`-ascent/-descent/-linespace/-fixed`，fixed 判据是 "iiiii" == 5×"i"）、
canvas text bbox（最长行测量宽 × 行距高，Tk 向外取整）、entry/spinbox
`-width`（字符数 × 实测 "0" 宽，Tk 的平均字符宽语义）。无 measurer 时
（headless/其它平台）全部回退原近似（8px/字符、size×0.6 等），测试用
fake measurer 覆盖。Windows/macOS 注册各自原生测量（DirectWrite/
CoreText）即可同享。

**菜单键盘遍历（2026-11-11，app 内菜单栏）**：Alt+助记符（cascade
`-label` 在 `-underline` 处的字母，大小写不敏感）下拉/收起对应 cascade；
下拉期间 Down/Up 在可选项间循环（跳过 separator 与 `-state disabled`）、
Return 执行聚焦项（cascade 则打开其子菜单）、Escape 关闭、Left/Right
在菜单栏相邻 cascade 间切换；键盘聚焦项按 hover 样式高亮。**绑定按
渲染作用域注册**：助记符只为菜单栏实际用到的字母、在菜单栏渲染期间
绑定（`When::Always`，entry 聚焦时也生效，同 Tk）；方向键/Return/
Escape 只在下拉渲染期间存在——nsh 的规则是同一按键"最新注册且可触发
者胜出"并吞掉按键，所以它们不能全局绑定（会抢走输入框光标键和
listbox/treeview 的 Up/Down 导航），下拉关闭即随作用域销毁、按键归还
控件。Apple 平台不绑定助记符（Option+字母是输入字符；Mac 无菜单
助记符惯例）；原生菜单栏（macOS/GTK/Win32）用各自的键盘遍历，不走此
路径。未覆盖：`tk_popup`/menubutton 弹出的菜单、子菜单内 Left 返回
上级。**事件域 wish 对照（2026-11-11，specs/tk/events.tcl，run.sh 第三遍）**：
`event generate` 驱动 bind 的全部 % 替换，与 wish 逐行精确比对（键类
两行豁免，见下）。修了三组：①`%s` 按十进制整数报（此前原样输出
`-state 0x4` 的 "0x4"；FocusIn/FocusOut/Map/Unmap 无 state 时报
`??`，同 Tk）；②`%k` 补 X keycode 表（Linux evdev/pc105：字母
a=38…z=52、数字 0=19…9=18、Return 36、space 65、Tab 23、Escape 9、
方向键/F 键等；表外 keysym 报 `??`——%k 本就平台相关）；③`%A`：
字符键报字符（Ctrl 折叠为控制字符 Ctrl+A=\x01，Shift 大写，Alt 不
影响），无字符键（方向键/F 键）报字面 `{}`，Return 报空；`event
generate` 未知名选项按 Tk 全列表报错（`-when/-data/-above/…` 十个
新增为接受忽略项）；`grab set/status/current/release` 已支持。**已
知差异（豁免）**：键类绑定的空 %A——Tk 按列表对象逐元素求值保留空
参数，rtcl 文本求值折叠（结构一致：字段数差一）；Xvfb 无 WM 时 Tk
的 Key 生成事件不派发（对照跑在真实显示上）。**字体度量对照 wish（2026-11-11，specs/tk/）**：`apps/rtcl-tk-demo
--bin tkprobe`（GTK 初始化 + 装后端跑脚本打印）与 wish 跑同一
`specs/tk/font-metrics.tcl`，`specs/tk/run.sh` 按容差比对（Tk 报
Xft.dpi 设备像素、rtcl 报 96dpi 逻辑像素，比例由 xrdb 的 Xft.dpi 给
出，本机 2.0；宽度容差 = 0.5px/字形 + 4%——两端各自按整数 hinting
取整；linespace 1.5px；ascent/descent 3px，Courier 的分割两库读表
不同但和一致；`tk scaling`/空串宽 0/`-fixed` 精确）。对照暴露并修
掉四组真 bug：①`font measure`/`metrics` 的族名解析只取首词——
`{DejaVu Sans Mono} 10` 测成了默认字体（渲染同受影响，重写为 Tcl
列表解析，花括号族名保词，未括的多词族名也连读，样式词按位置识别
——"roman" 在尺寸前是族名）；②负 `-size`（像素）分支不可达（
`-16` 先被当数解析取绝对值成 16 磅，大 33%），现 px/磅 = 96/72；③
`-fixed` 判据 "iiiii"==5×"i" 对任何字体恒真（同字形线性），改
"iiiiiiiiii"=="WWWWWWWWWW"；④`tk scaling` 默认 1.0 → 4/3（96dpi，
与渲染器的磅→像素一致），`winfo fpixels . 1i` = 96。回归的
`font actual` 差异一并修：接受任意描述（此前仅命名字体，报错）、
`-family` 报后端解析后的真实 face（新 `set_font_family_resolver`
pango 钩子，Helvetica→Nimbus Sans）、输出按 Tk 选项序、尺寸取整为
磅、`-slant italic`/`-underline/-overstrike` 全支持；`font create`
返回名字 + Tk 自动命名 `font1...`（此前返回空）、裸字体基础选项
`-family {} -size 0`、标准九字体（TkDefaultFont/TkTextFont/
TkFixedFont/TkMenuFont/TkHeadingFont/TkCaptionFont/
TkSmallCaptionFont/TkIconFont/TkTooltipFont，generic 族名 sans-serif
/monospace）进 `font names`、可 configure（首个改动拷贝入 rt.fonts）、
可删除（删除标准字体不复活）；`font families` 经 pango（298 个 vs
Xft 377 个，枚举差异）；`font configure` 无参按 Tk 顺序报配置值、
单选项查询、错误消息同 Tk（bad option/bad -weight/named font ...
doesn't exist/already exists）。`font actual` 的 `-family` 对
"Courier" 报 pango 规范名 Nimbus Mono PS（Tk 保留请求名）——单一家
族命名差异，记录在案。度量对照 127/127 在容差内；九标准字体的
actual/fixed 与 wish 逐字节一致。**几何域 wish 对照（2026-11-11，specs/tk/geometry.tcl +
compare-geo.tcl，已并入 run.sh 第二遍）**：canvas text bbox、text
`dlineinfo`（三条真实行 + 越界行）、entry/listbox `winfo reqwidth`，
三种字体。发现并修：`dlineinfo` 此前按控件列数×7 报宽度、18px 行高
14px ascent 常数——现按该显示行文本实测（宽度/linespace/ascent，
x/y = 边框+内边距 1.5 逻辑像素；无测量器回退 8px/字符、18/14）；
canvas text bbox 补 Tk 的水平 ±1 设备像素外扩（逻辑空间 ±0.5，与
canvas_bbox 的 floor/ceil 出向取整配合后与 Tk 在 1×/2× 显示都一致）；
entry/listbox/text 的 `winfo reqwidth` 字符宽度改实测 "0" 宽（text +3、
listbox +2、entry +3 逻辑像素 chrome，与 wish 的 reqwidth 对上）。
跨实现已知差异（对照豁免并记录）：Tk 8.6 的 `dlineinfo` baseline 对
所有行恒报首行值（y 走 3/31/59 而 baseline 恒 20，其自身缺陷），
rtcl 按数学定义 y+ascent；canvas text 宽度在 HiDPI（Xft.dpi=192）下
Tk 报 canvas 单位的 2× 溢出（其著名缺陷），比较器按字体缩放归一。
单测 canvas bbox 期望随 ±0.5 外扩更新。**text 高级特性（2026-11-11）**：`tag cget` 对已知未设选项返回 Tk
默认值（-offset/-overstrike/-lmargin*/-spacing*/-elide/-underline 等为
0、-justify left、-relief raised、-tabstyle tabular）；扩展选项
（-tabs/-offset/-overstrike/-lmargin*/-rmargin/-spacing*/-tabstyle）
模型层存储/查询完整——渲染投影只覆盖颜色/字号/粗斜体/下划线（
ProseMirror mark 语义之外的是布局属性，富文档无法表达，视觉近似）。
`text search` 补 `-elide` 开关：默认跳过完全落进 -elide 标签的匹配
（部分重叠同样跳过），`-elide` 才搜索隐藏文本；清理了两处遗留的
SEARCH 调试输出。**wm 窗口管理（2026-11-11）**：`wm attributes` 实现通用子集
（`-alpha`（0–1 截断，默认 1.0）、`-fullscreen`、`-topmost`、`-type`、
`-zoomed`、`-modified`、`-toolwindow`；无参返回全名值对），改动经
`set_window_attribute_hook` 通知后端——nsh 后端把 `-fullscreen`
映射 `toggle_full_screen`、`-topmost` 映射 `set_level(Floating/Normal)`
（仅主窗口 "."，模型里的次级 toplevel 是窗口内 overlay）；Linux GTK
可能忽略 level，Windows/macOS 生效。`wm stackorder` 按 tree 子序
（raise 追加=置顶、lower 前插=置底）报从低到高的 toplevel 列表，
支持 `isabove/below` 切片；`wm overrideredirect` 真实读写。`-alpha`
暂无渲染通道（fork 无该桌面命令）。**平台扩展点**：原生对话框/剪贴板由宿主回调注入
`set_message_box_handler/set_file_dialog_handler/set_color_dialog_handler/
set_dialog_handler/set_clipboard_getter/set_clipboard_sink`；另有
`set_native_event_pump`（vwait 嵌套泵）、`set_screen_metrics`
（`winfo screenwidth/height`）、`set_bell`
（显示蜂鸣）。内置实现有 Linux GTK4 与 **Windows Win32**（本仓库验证环境）：
Windows 的 `tk_messageBox` 是 `MessageBoxW`（`-type` 六组按钮、`-icon`
四种、`-default` 映射 DEFBUTTON，Esc/关闭返回 cancel——与 Tk 在 Windows
上的实现同源）；`tk_getOpenFile/tk_getSaveFile` 是
`GetOpenFileNameW/GetSaveFileNameW`（`-filetypes` 转 filter 串、
`-multiple` 解析多选 buffer、`-confirmoverwrite`/`-defaultextension`/
`OFN_NOCHANGEDIR` 保持 cwd，返回路径转正斜杠——`file normalize` 的形态）；
`tk_chooseDirectory` 是 `SHBrowseForFolderW`（BFFM_SETSELECTIONW 定位
`-initialdir`，apartment 可用时带新建目录样式）；`tk_chooseColor` 是
`ChooseColorW`（十六自定义色跨调用保留，返回 `#rrggbb`）；`tk_dialog`
是内存 `DLGTEMPLATE` + `DialogBoxIndirectParamW`（消息在顶、按钮在底、
default 获焦点、Esc/关闭返回 default——经典模板不需要 comctl32 v6；
坑：经典模板的 DS_SETFONT 字体块只有 pointsize+typeface，几何忘传
会得到 0×0 细条窗）；剪贴板 getter 读
`CF_UNICODETEXT`；screen metrics 用 `GetSystemMetrics`；bell 用
`MessageBeep`；vwait 泵走 `PeekMessageW/DispatchMessageW`。
**Windows 实机验证（2026-11-11，Vagrant Win11 VM，
x86_64-pc-windows-gnu + MSYS2）**：宿主机交叉编译整个 app（`cargo build
-p rtcl-tk-app-native --target x86_64-pc-windows-gnu`，>0 错误即失败，
strip 后仅依赖系统 DLL），截图确认原生 Win32 标题栏、`SetMenu` 菜单栏
（File 菜单）、`tk_messageBox -type ok -icon information` 弹出
MessageBoxW（标题/正文/图标/OK 全对）、`tk_dialog` 弹出模板
DialogBox（三按钮、default 获焦点）、entry `-width` 字符宽度下限生效
（此前 Windows/Android 过窄：pack 非 fill 收缩时 w_full 塌缩，
现按 ~8px/字符给 min_w，GTK 填充行为不变）、label `-image` 与 text
内联 image 均绘制（固有尺寸修复生效）。构建坑见仓库外记录：WinRM shell
默认 4GB 内存配额会杀死并行 rustc（构建须经计划任务跑）、fork 的
WebView2 静态库是 MSVC 专用（fork 49b1afc 起非 MSVC 懒加载
WebView2Loader.dll）。macOS 宿主可自行注册平台原生实现；未注册时对话框回退默认
按钮/空路径/初始色、screen 1920×1080、bell 静默。IME 与基础 a11y 由 nsh
原生 input/RichEditor/按钮承担；自绘控件补了关键标签（spinbox
Increment/Decrement、toplevel Close window），Canvas/Chart 仍是纯绘制、
无逐项 a11y 语义。**Android port 进展**：本机有 SDK（`~/android-sdk`，NDK 29）+ cargo-ndk
4.1.2；tk 全栈（core/runtime/nsh/demo）已通过
`cargo ndk -t arm64-v8a --platform 24 check`（serde_json 已改为无条件依赖，
android/windows 均可用）。fork 的 nsh-platform-android 完整存在
（JNI libnsh.so + Java 运行时），nsh-cli 的 android.rs 能打 APK
（classes.dex + libnsh.so per ABI + AppActivity 跑 Rust root）。
Android app crate 已建：`apps/rtcl-tk-app`（nsh.json `"rust": "native"`
+ `root: TclApp`；native crate 依赖 rtcl-tk-demo 的脚本装载 +
`#[native_module]` 注册 TclApp；nsh 依赖直连 fork git 源——patch 不随
生成的 .nsh/rust workspace 传播；demo 脚本经 `__DEMO_PNG__` 替换注入
`demo_png_path()`——路径统一正斜杠（Windows 反斜杠在 Tcl 词里是转义）、
Android 写进程包名自己的 files 目录）。`nsh run --target android` 端到端验证通过：装
`platforms;android-35`（javac classpath）与
`system-images;android-28;default;x86_64`（arm64 镜像在 x86 主机不被
QEMU2 支持）+ emulator 37.2 + AVD 后，出 APK（dev.rtcl.tkapp）安装并
启动，Tcl 驱动的 label/entry/button 渲染正常，点按后 `-command` 改
label "Tapped!"（docs/images/tk-android*.png）。宿主钩子（对话框/
剪贴板）android 侧未注册，回退默认值。**图片复验通过**（固有尺寸
修复后）：label -image 与 text 内联 image 在 x86_64 模拟器均绘制；
菜单以 app 内菜单栏呈现（Android 不经桌面 runner，未开原生菜单）。

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
