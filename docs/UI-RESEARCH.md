# rtcl UI 调研：Jupyter widget 生态 × 数据分析/运维需求

状态：调研纪要（指导 L1 组件规划与协议演进）。配合 docs/UI-DSL.md 阅读。

## 0. 出发点

项目初衷：复刻 Sentry + 日志分析类产品。此类产品 UI 的复杂度根源不是
组件数量，而是**固定信息架构试图预建所有查询路径**——issue 列表、
event 详情、trace waterfall、release 对比、自定义 dashboard，全部由
产品预判排障路径。实际排障是**探索式对话**：下一步看什么取决于上一步
看到什么。

rtcl 的 hypothesis：以 shell 为轴（CLI 的探索式模型），UI 作为
**按上下文即时生成、用完可弃的投影**（`ui::surface` 建、
`ui::destroy` 弃），开发成本与易用性可同时成立。本调研验证这个方向
在 Jupyter 生态（最成熟的"代码驱动 UI"生态）中的对应物与经验教训。

## 1. Jupyter widget 生态

### 1.1 ipywidgets 协议架构

核心模型（[widget messaging protocol](https://github.com/jupyter-widgets/ipywidgets/blob/master/packages/schema/messages.md)）：

```
kernel (Python)                    frontend (浏览器)
widget model ──comm open────────→  model 镜像
   │ state 变更 (traitlets sync=True)│
   ├────── comm msg: update ──────→ 视图重渲染
   │←───── comm msg: 事件/回写 ─────┤
```

- **状态同步优于消息**：traitlets `sync=True` 的字段是单一事实源，
  任意时刻可从 state 完全重建前端——不依赖 kernel 存活、与消息到达
  顺序无关。custom messages（`widget.send()`）则要求前端已渲染、
  kernel 在线，顺序敏感（[anywidget: The Good Parts](https://anywidget.dev/en/jupyter-widgets-the-good-parts/)
  明确建议 prefer traitlets over custom messages）。
- **事件回调的错误被静默吞掉**是 ipywidgets 的著名坑，社区 workaround
  是用 `Output` widget 包住回调。我们在协议层解决：`pump_events`
  收集错误，`dispatch_event` 向 JS 上抛（UI-DSL.md §6）。
- **`observe` 必须指定 names**：否则内部 trait 变化也触发回调，
  一次逻辑变更多次回调 → 抖动。对应我们的 `props.bind` 精确路径
  订阅（模型路径 → widget 集合），天然无此问题。
- **二进制通道**：消息协议支持 buffer_paths，bytes ↔ DataView
  直接过线不过 JSON/base64。我们当前 op 协议只有 JSON——见 §4。

### 1.2 anywidget：开发体验的收敛方向

[anywidget](https://anywidget.dev/blog/introducing-anywidget/) 把自定义
widget 收敛为：一个 Python 类 + 一个 ESM 文件（甚至内联字符串），
导出 `render({model, el})`（每视图一次）与可选 `initialize({model})`
（每模型一次）。无打包、无构建链，现已成为官方推荐的自定义 widget
方式。

对照 rtcl：

| anywidget | rtcl ui::* | 状态 |
|---|---|---|
| ESM 渲染器无构建 | ui-runtime.js 纯 ESM，`registerRenderer` 注册 | 已对齐 |
| model state 单一事实源 | Tcl 侧模型树 + op 同步 | 已对齐 |
| render 每视图一次 | widget path 唯一 → 单视图 | 更简单，够用 |
| traits 声明 + 校验 | catalog spec（kind/meta/default）+ Tcl 侧校验 | 已对齐 |
| `to_json` 自定义序列化 | `_json_or_string`（JSON 原样/字符串编码） | 已对齐，缺二进制 |

### 1.3 "代码生成 UI"的谱系另一端：Streamlit / Solara / Panel

- Streamlit：状态变化 → **整个脚本重跑**。入门最简单，但
  [随复杂度增长需要越来越多 workaround](https://shiny.posit.co/py/docs/comp-streamlit.html)。
  这是"无结构的即时 UI"的极限——我们不走这条路（rtcl 是结构持久 +
  op 增量更新）。
- [Solara](https://itnext.io/webapps-in-python-with-solara-a-streamlit-killer-ab6fcc7bf5d7)：
  ipywidgets + Reacton，响应式变量驱动**局部**重渲染；可组合组件。
- [Panel](https://panel.holoviz.org/explanation/comparisons/compare_streamlit.html)：
  显式绑定 widget/plot 到计算，只重跑受影响部分。

rtcl ui::* 的位置：**显式结构命令（词表）+ 模型绑定订阅**——比
Streamlit 持久、比 Solara 少一层框架概念，代价是命令式（LLM/规则
生成时这恰是优点：op 序列即可审计的 transcript）。

## 2. 数据分析场景需求

### 2.1 大表格是硬门槛

- [ipydatagrid](https://github.com/jupyter-widgets/ipydatagrid/issues/204)
  在十万行级仍有初始化/渲染性能问题；社区答案是
  [Perspective](https://github.com/perspective-dev/perspective)——浏览器内
  流式分析引擎，数据走 [Apache Arrow IPC](https://perspective-dev.github.io/guide/explanation/table/loading_data.html)，
  pivot/聚合在客户端完成。[ClickHouse 的官方实时可视化链路](https://clickhouse.com/blog/streaming-real-time-visualizations-clickhouse-apache-arrow-perpsective)
  即 ClickHouse→Arrow→Perspective。
- 结论：`ui::table` 必须**虚拟滚动 + 分页/游标喂数**（Tcl 侧
  `ui::model set /rows/...` 分批），不能一次 JSON 全量。百万行场景
  留 Arrow 通道（§4），不阻塞 L1 落地。

### 2.2 图表

Vega-Lite spec 即契约（已在 UI-DSL.md §2.3 `ui::chart -vegalite`）。
流式场景用 model 增量 append + spec 不变的方式（Perspective 经验：
**图表定义与数据分离**，数据走模型通道）。

### 2.3 数据分析组件优先级

| 组件 | 场景 | 借鉴 |
|---|---|---|
| `ui::table` | 查询结果、聚合表 | ipydatagrid 的列定义 + 虚拟滚动 |
| `ui::chart` | 指标、分布 | Vega-Lite（声明式 spec 过桥） |
| `ui::kv` | 单行详情、上下文变量 | — |
| `ui::tree` | JSON/嵌套结构 | json-render 系 |

## 3. 运维（Sentry 类）场景需求

Sentry 视图清单的拆解——哪些是我们的 L1 组件、哪些被 shell 模型取代：

| Sentry 视图 | 本质 | rtcl 对应 |
|---|---|---|
| issue 列表（fingerprint 聚合） | 分组计数表 | `ui::table`（服务端聚合好喂入） |
| event 详情 | KV + 嵌套上下文 | `ui::kv` + `ui::tree` |
| stack trace | 帧列表 + 源码上下文 | `ui::tree`（帧为节点） |
| trace waterfall | span 树 + 时间轴 | `ui::trace`（L1 新组件，见下） |
| release/趋势图 | 时序 | `ui::chart -vegalite` |
| 日志流 | 追加式带级别着色 | `ui::log -path /log`（增量 append） |
| 搜索栏（结构化查询） | DSL 输入 | **不需要组件**——shell 本身就是 |
| 自定义 dashboard | 持久布局 | **不需要**——surface 即建即弃，脚本可存档重放 |

后两行是 hypothesis 的核心兑现：Sentry 复杂度的大头（查询构建器 UI、
dashboard 编辑器 UI）在 shell 模型下**直接消失**——查询是 Tcl 命令，
dashboard 是一段可重放的脚本。

### 3.1 trace waterfall（唯一需要新渲染原语的）

span 树 `{name, start_ms, dur_ms, children[]}`，渲染为甘特式时间轴。
纯 DOM 可做（div 宽度百分比定位），L1 组件 `ui::trace -path /trace`。
Vega-Lite 也能画甘特但交互（折叠/缩放）弱，先独立组件。

### 3.2 日志流

`ui::log -path /deploylog`：模型值是行数组，append 优化（op 协议已有
`model set` 到子路径 `/deploylog/42`，渲染器 diff 追加节点而非全量
重渲染）；级别字段着色、正则高亮、跟随/暂停滚动。这是运维场景
使用频率最高的组件。

## 4. 对协议与实现的建议（按优先级）

**P0（L1 首批组件，纯 JSON 协议内可完成）**
1. `ui::table`（列定义 + 虚拟滚动 + 行事件 `-rowactions`）
2. `ui::log`（流式追加渲染器）
3. `ui::kv` / `ui::tree`（详情视图双件套）

**P1**
4. `ui::chart -vegalite`（spec 过桥，数据走 model）
5. `ui::trace`（span 树甘特图）
6. `ui::confirm`（模态动作，-command 回调形式——浏览器纪律 §4.2
   禁止 vwait 等返回值）

**P2（协议演进，等真实瓶颈再做）**
7. **二进制模型通道**：op JSON 外挂 Arrow IPC buffer（参考
   ipywidgets buffer_paths）。`ui::table` 到十万行以上才需要。
8. `ui::query` op（异步问 DOM：滚动位置、几何）——Worker 化之后
   才有意义（主线程形态 JS 直接知道答案，契约 §3 已留位）。
9. surface 布局管理（分屏/标签页——L1 `ui::tabs` 已在契约）。

**明确不做**
- 全量 rerun 模型（Streamlit 式）——与结构持久原则冲突。
- widget 多视图（ipywidgets 的 model-view 1:N）——path 唯一即可，
  需要展示两份就建两个 path 绑同一模型路径。
- 富文本 markdown `-text`（M5 候选，安全风险：渲染 HTML 需 sanitize）。

## 5. 反模式清单（调研中确认的坑，写代码时自查）

1. 回调错误静默（ipywidgets observe 坑）——我们 pump_events 上抛，保持。
2. 粗粒度变更通知（observe 不带 names）——bind 精确路径订阅，保持。
3. 全量状态重传（Streamlit rerun）——op 增量 + model 子路径更新，保持。
4. 顺序敏感的 custom message——我们的 op 是**可重放日志**，任何时刻
   `ui::destroy .s` + 重放 ops 可重建，保持这个性质（快照能力隐含其中：
   JS 侧可从 op 日志重建任意 surface）。
5. 一次性全量喂大数据——table/log 从第一版就要游标/追加式 API。
