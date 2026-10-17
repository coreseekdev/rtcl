# Tk 描述兼容 vs 改良 ui::*：路线决策记录

状态：决策建议（待确认）。触发：visualtkinter.com 设计器的调研。
配合 docs/UI-DSL.md / UI-LAYOUT.md 阅读。

## 1. 调研对象

| 对象 | 形态 | 观察 |
|---|---|---|
| [visualtkinter.com](https://visualtkinter.com/designer?template=calculator) | 在线拖拽设计器 | 生成的 calculator 模板是**纯 `place` 绝对定位**（每按钮 `place(x=20,y=80,width=50,height=50)`）——GUI builder 的 WYSIWYG 天性 |
| [VisualTK 离线备份](https://github.com/mechatronod/VisualTK)（visualtk.com 的 HTTrack 快照） | 已在本地跑通（见 §5） | 经典三栏：控件调色板 / 点阵画布 / **Tkinter option 全集平铺的属性检查器**（Activebackground/Anchor/bitmap/compound/cursor…几十个属性无分组无校验） |
| [tcltk.wasm](https://github.com/t3tra-dev/tcltk.wasm) | Tcl/Tk 8.6.18 经 Emscripten 移植浏览器 | 真 Tk 渲染（canvas 输出），2026 年活跃维护 |
| pygubu / PAGE / Tkinter-Designer | Tkinter RAD 生态 | 同样以 place/绝对坐标为主输出 |

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

## 3. 关键论据：设计器工具暴露了 Tk 词表的真实成本

VisualTK/visualtkinter 的 generated code 全部是 place 绝对定位，
检查器是 option 全集平铺——这不是工具做得差，而是 **Tk 词表缺少
中间层**：没有布局语义的结构化声明（pack 顺序即语义），没有
属性约束（任何 option 任何值都合法到运行时才炸），没有数据绑定
（-textvariable 是弱形式）。

机器生成场景下这三个缺失是致命的（LLM 需要校验反馈回路），
人工设计器场景下它们退化为"画布拖拽 + 大表单"。我们的 catalog
spec（kind/meta/default + 布局公共选项）恰好同时是：

1. 词表命令的校验器（已实现）
2. **LLM 的 prompt 词表**（`ui::catalog` 内省即文档）
3. **属性检查器的表单 schema**——kind=enum→下拉、bool→勾选、
   size→带单位输入、script→代码框。VisualTK 右侧检查器要硬编码
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
