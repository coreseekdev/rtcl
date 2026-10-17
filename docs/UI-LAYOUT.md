# rtcl UI 布局机制：参照系调研与词表设计

状态：设计定稿（v1 布局选项已实现）。配合 docs/UI-DSL.md / UI-RESEARCH.md 阅读。

## 1. 四条参照系

### Qt / QML

QML 内部就分两层（[RowLayout 文档](https://runebook.dev/zh/docs/qt/qml-qtquick-layouts-rowlayout)）：

- **Positioners**（Row/Column/Grid/Flow）：只管排列，不做尺寸协商；
- **Layouts**（RowLayout/ColumnLayout/GridLayout）：真正的布局管理器，
  子项用**附加属性**声明空间意图：

| QML 附加属性 | 语义 |
|---|---|
| `Layout.fillWidth/fillHeight` | 占满该轴剩余空间 |
| `Layout.preferred/minimum/maximumWidth/Height` | 尺寸提示与约束 |
| `Layout.alignment` | 单元格内对齐（Qt.AlignHCenter 等） |
| 容器 `spacing` | 子项间距 |

Qt Widgets 侧（QLayout + QSizePolicy + stretch factor）是同一思想的
命令式版本。要点：**子项声明、父容器解释**的父子契约。

### Tk（血缘最近——ui::* 的路径语法 `.s.main.inc` 即 Tk 风格）

三个布局管理器，与 widget 正交（widget 不知道自己在哪个管理器里）：

- `pack .x -side top -fill both -expand 1` —— 一维堆叠。`-fill` 决定
  widget 是否填满**分配给它的槽**（none/x/y/both），`-expand` 决定
  槽是否**争抢父容器的剩余空间**。两个正交概念。
- `grid .x -row 0 -column 1 -sticky nsew` + `grid columnconfigure . 1 -weight 1`
  —— 二维网格，列/行的 `weight` 即剩余空间分配比例。
- `place` —— 绝对/相对坐标，极少用。

### Web 标准：CSS 本身就是那个标准

Web 上"类似 QML Layouts 的规范"不是某个组件库，而是 W3C 的 CSS 布局
模块本身——浏览器内置布局引擎，我们不需要实现任何算法：

| CSS 模块 | 对应 | 状态 |
|---|---|---|
| Flexbox | Tk pack / QML RowLayout（一维） | 全浏览器 |
| Grid | Tk grid / QML GridLayout（二维） | 全浏览器 |
| [Anchor Positioning](https://www.w3.org/TR/css-anchor-position-1/) | QML anchors（锚点相对定位） | W3C spec，主战场是 tooltip/popover 叠层，非主布局 |
| Container Queries | 响应式组件内断点 | 全浏览器 |

### 独立布局引擎（desktop 化时的路标）

[Yoga](https://www.yogalayout.dev/)（Meta，C++，React Native 的布局
引擎）和 [Taffy](https://github.com/dioxusLabs/taffy)（Rust，Dioxus/
Bevy/Servo 系采用）把 CSS Flexbox/Grid/Block 算法做成无 DOM 的独立
计算库。结论：**即使将来脱离浏览器，布局算法仍然用 CSS 语义**——
Taffy 就是现成的 Rust 实现。DSL 层语义与 CSS 对齐是全平台最安全的投资。

## 2. 设计决策

1. **布局算法 = CSS**（web 形态）；DSL 只暴露语义选项，渲染器映射到
   `el.style`。不自研算法，不引入 Yoga/Taffy（除非 desktop 化）。
2. **词表词汇取 Tk 的**（fill/expand 语义群），因为它与 ui::* 路径
   语法同宗，且 fill（填槽）与 weight（争空间）的正交性比
   QML 的 fillWidth 布尔更精确。
3. **声明位置在子项上**（QML 附加属性风格），与组件创建同一条命令
   ——对 LLM/规则生成友好（一条 op 完成创建+布局），不像 Tk 那样
   有独立的 pack/grid 命令要维护顺序。
4. **布局选项是所有组件的公共选项**，catalog 内置，不进各组件 spec；
   JS 运行时统一从 props 提取应用，组件渲染器不感知。

## 3. 词表（布局选项集 v1）

所有 L0/L1 组件通用：

```
-fill    none|x|y|both   填满分配槽（交叉轴 → align-self:stretch；
                          主轴填满须配合 -weight，flex 无"槽"概念）
-weight  n               剩余空间争抢比例（Qt stretch / Tk grid weight /
                          CSS flex-grow 的统一抽象；0 = 不争抢）
-width   size            尺寸提示（px，或带 %/em/rem/ch/vw 单位）
-height  size
-align   start|center|end|stretch   交叉轴对齐（align-self）
```

容器附加（Row/Column/Card/Surface/未来的 Grid）：

```
-gap     size            子项间距（gap）
-pad     size            内边距（padding）
```

映射表（JS 运行时实现）：

| 词表 | CSS |
|---|---|
| `-weight n` | `flex: n 1 0` |
| `-fill y`（Row 内）/ `-fill x`（Column 内） | `align-self: stretch` |
| `-fill both` | 上者 + `flex-grow: 1`（若未显式 -weight） |
| `-align v` | `align-self: v` |
| `-width/-height s` | `width/height: s` |
| `-gap/-pad s` | 容器 `gap/padding: s` |

示例：

```tcl
ui::row .s.bar -gap 8
ui::text .s.bar.title -text Pods -weight 1      ;# 占满中间
ui::button .s.bar.refresh -text ⟳ -align center
ui::text .s.bar.log -text ... -width 30% -fill both
```

## 4. 边界与后续

- **Grid（二维）**：L1 加 `ui::grid` 容器 + 子项 `-row/-col/-span`，
  直接映射 CSS Grid（grid-row/grid-column）。Row/Column 嵌套可覆盖
  大多数运维面板，Grid 不阻塞 L0。
- **Anchor Positioning**：用于 tooltip/popover 叠层（L2 的
  `ui::popover` 类组件），主布局不用。
- **响应式**：运维面板宽度相对固定，v1 不做容器查询；需要时
  Container Queries 是纯 CSS 侧的事，DSL 无感。
- **desktop 化**：词表语义与 CSS 对齐后，换渲染器时接 Taffy 即可，
  词表不变（契约 §3 的"op 协议平台中立"原则覆盖布局层）。
