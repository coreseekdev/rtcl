# Tk 描述兼容 vs 改良 ui::*：路线决策记录

状态：决策建议（待确认）。触发：visualtkinter.com 设计器的调研。
配合 docs/UI-DSL.md / UI-LAYOUT.md 阅读。

## 1. 调研对象

| 对象 | 形态 | 观察 |
|---|---|---|
| [visualtkinter.com](https://visualtkinter.com/designer?template=calculator)（线上现行版） | 在线拖拽设计器 | **默认 grid 推断模式**（见 §3.1）：从画布位置推断 row/column 轨道、columnspan、sticky，并生成 `columnconfigure weight=1` + minsize；可切回 fixed（place）。CustomTkinter 导出、项目级 Style、LAYERS 面板、GRID 吸附——与老站快照是两个时代的产品 |
| [VisualTK 离线备份](https://github.com/mechatronod/VisualTK)（visualtk.com **老站** 2023/24 快照，与线上 visualtkinter.com 差距大，勿混为一谈） | 已在本地跑通（见 §5） | 经典三栏：控件调色板 / 点阵画布 / **Tkinter option 全集平铺的属性检查器**；纯 place 输出 |
| [tcltk.wasm](https://github.com/t3tra-dev/tcltk.wasm) | Tcl/Tk 8.6.18 经 Emscripten 移植浏览器 | 真 Tk 渲染（canvas 输出），2026 年活跃维护 |
| pygubu / PAGE / Tkinter-Designer | Tkinter RAD 生态 | 以 place/绝对坐标为主输出（未逐一复核现行版） |

## 2. 三条路线

**A. tcltk.wasm（真 Tk 跑在浏览器）**
- 得：100% Tk 兼容，存量 wish 脚本直接跑。
- 失：canvas 渲染 ≠ DOM——无 CSS、无选择器、无 a11y，字体/合成
  全自己扛；Tk 组件是 90 年代桌面隐喻；与我们 web shell 的
  DOM+CSS 变量主题完全两套栈；Emscripten 体积。
- 判：**不取**。它回答的是"如何在浏览器跑 Tk"，而我们的问题是
  "如何生成运维 UI"——后者要的是 web 原生渲染。

**B. Tk 词表兼容层，建在 ui::* op 协议之上**
```tcl
button .s.b -text Hi -command {...}   ;# → ui::button .s.b ...
pack .s.b -fill both -expand 1        ;# → ui::configure .s.b -fill both -weight 1
```
- 一层 ~200 行 Tcl proc 的翻译皮，op 协议与渲染器不变。
- 得：Tk 用户零迁移成本；教程/存量脚本语义可复用。
- 失：Tk option 全集（-bg/-fg/-font/-relief/-bd…）与 headless
  主题哲学（样式归 CSS）冲突，必须裁剪为白名单子集；
  pack 的顺序敏感语义（先 pack 先贴边）在结构化 Row/Column 里
  消失——兼容只能是**词汇级**而非**行为级**。
- 判：**可选皮肤，后置**。等出现真实需求（存量 Tk 脚本/强 Tk
  习惯的运维团队）再做，成本可控。

**C. 改良 ui::*（现状主线）**
- catalog 校验、模型绑定、batch、内省——为机器生成（LLM/规则）
  设计的词表，人工直写同样顺手。
- 判：**主线，继续**。

## 3. 关键论据：Tk 词表缺的不是布局语义，而是中间层元数据

~~设计器只会生成 place~~（已被证伪，见下）——真实情况是：
**布局语义 Tk 有（grid），设计器也能产出；Tk 词表真正的缺口在
元数据与数据层**。

### 3.1 visualtkinter.com 现行版的 grid 推断（2026-10 复核）

calculator 模板在 grid 模式下生成：

```python
self.display.grid(row=0, column=0, columnspan=4, sticky="ew", padx=4, pady=4)
self.button1.grid(row=1, column=0, sticky="w", padx=4, pady=4)
...
root.columnconfigure(0, minsize=58, weight=1)   # 轨道推断 + weight + minsize
root.rowconfigure(0, minsize=48)
```

推断算法四件套：**画布位置 → 网格轨道；跨度 → columnspan；保守
sticky（按钮 "w"、display "ew"）；画布尺寸 → minsize + 均分
weight**。这证明两件事：

1. Tk grid 语义（row/column/span/sticky/weight/minsize）完全可以
   由设计器产出——我们 L1 `ui::grid` 的词汇与 Tk 对齐（
   `-row/-col/-span/-sticky/-weight`）因此多了一条理由：该词汇
   已被设计器场景验证为可可视化、可推断。
2. "画布自由拖拽 + 网格轨道推断"是设计器交互的已验证可行解，
   将来若做设计器形态可直接参考（而不是约束用户逐格摆放）。

### 3.2 仍然成立的缺口

但线上版同时确认了 Tk 词表的另一半问题依旧存在：

- **无 catalog/校验概念**：检查器的属性 schema 是每个设计器工具
  侧硬编码维护的（线上版分组精美，但那是工具自己的元数据，
  与 Tk 运行时无关；运行时对任何 option/值组合照旧宽容到出错
  才炸）。我们的 catalog 是单一事实源，同时服务校验/LLM/检查器。
- **无数据绑定**：生成代码里显示刷新靠 `entry.insert/delete`
  命令式操作，模型概念缺失。
- **无批量化/事务**：逐 widget 命令流。

机器生成场景下这三点是致命的（LLM 需要校验反馈回路），而它们
恰恰是 ui::* catalog 的既有能力：

1. 词表命令的校验器（已实现）
2. **LLM 的 prompt 词表**（`ui::catalog` 内省即文档）
3. **属性检查器的表单 schema**——kind=enum→下拉、bool→勾选、
   size→带单位输入、script→代码框。Tk 设计器的检查器要硬编码
   几十个属性，我们一行 `ui::catalog Button` 全导出。

第 3 点是自举机会：用 ui::* 渲染 ui::* 的属性检查器
（`ui::form -schema [ui::catalog X]` 的 L1 形态），
把"设计器"从独立工具变成 shell 里的一条命令。

## 4. 建议的后续动作

1. 主线 C 继续：L1 组件按 UI-RESEARCH.md §4 优先级推进。
2. B（Tk 皮肤）冻结到出现真实拉动信号；届时只翻译白名单 option
   子集（text/command/布局群），样式 option 一律拒绝并提示用主题。
3. 设计器形态以"catalog 驱动检查器"自举方式跟进（L1 `ui::form`
   落地后顺手做 demo），不引入外部拖拽工具。

## 5. 离线包运行记录

```bash
# 已在 .refer/visualtk/（gitignored，参考资料不入库）
cd .refer/visualtk/site/VisualTK_com
python3 -m http.server 8766 --bind 127.0.0.1
# 入口：http://127.0.0.1:8766/web.archive.org/web/20240418073728/https_/visualtk.com/index.html
```
