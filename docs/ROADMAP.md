# rtcl 应用平台路线图：支撑 Sentry / ELK / Splunk / Jupyter 类应用

状态：规划（v1，待逐块启动）。前置：docs/UI-DSL.md（契约）、
UI-RESEARCH.md（需求）、UI-LAYOUT.md（布局）、UI-TK-COMPAT.md（词表路线）。

## 0. 四类应用的本质抽象

| 产品 | 本质 | 落在我们架构上的形态 |
|---|---|---|
| Sentry | 事件摄取 → fingerprint 聚合 → 详情钻取 → 告警 | ingest + `ui::table` + `ui::kv/tree/trace` + 规则 proc |
| ELK/Splunk | 日志摄取 → 索引 → 查询语言 → 时间直方图 + 日志流 | ingest + 查询词表 + `ui::chart` + `ui::log` |
| Jupyter | REPL + 富输出 + widget 交互 + 可分享文档 | web shell（已有）+ ui::* 面板（富输出）+ 脚本即 notebook |
| 共性 | **查询 → 投影 → 钻取** 的探索式循环 | shell 为轴、surface 即建即弃（hypothesis 本身） |

一个关键差异化判断：**Splunk SPL / KQL 是字符串 DSL，而 Tcl 本身就是
查询语言**。`log::search "level:error" | stats count by service` 这类
管道词表是"可组合的 Tcl 命令"而非新语法——这是 rtcl 对位 SPL 的
结构性优势，也是 L2 的核心工作块。

## 1. L0 引擎与宿主（基础层）

| 工作块 | 状态 | 验收信号 |
|---|---|---|
| Tcl 解释器 parity + 性能 | ✅（judge 4072/4072，反超/持平 tclsh） | 保持 |
| wasm 宿主 + 事件回路（HostClock/post_event/dispatch_event） | ✅ | 保持 |
| ui::* L0 词表 + op 协议 + 布局 v1 | ✅ | 保持 |
| **Worker 形态**（解释器入 Worker，op 走 postMessage） | 缺失（M5） | UI 线程零阻塞；`after`/`vwait` 纪律完整生效；demo 页切 Worker 后行为不变 |
| **safe interp 应用策略**（LLM/规则代码隔离：UI 权限按域授予、max_steps） | 部分（interp 命令与 set_max_steps 已有） | 生成代码在 child interp 跑，越权 UI 操作报结构化错误 |
| 性能收尾（TODO.md 剩余杠杆） | 进行中 | 与 UI 无耦合，按原节奏 |

## 2. L1 UI 组件（投影层）

按 UI-RESEARCH.md §4 优先级（P0 先行）：

| 工作块 | 状态 | 关键设计点 | 验收信号 |
|---|---|---|---|
| `ui::table` | ✅ v1 | 列定义 + **虚拟滚动 + 游标喂数**（禁一次性全量）；`-rowactions` 行事件 | 10 万行流畅滚动；模型子路径增量追加 |
| `ui::log` | ✅ v1 | 流式追加渲染器 + 级别着色 + 跟随/暂停（正则高亮留 §8）；`-path` 行数组游标追加 | 2 万行 756ms（~26k 行/s）；上滚暂停/回底恢复 |
| `ui::kv` / `ui::tree` | ✅ v1 | 详情双件套；kv `-keys` 键序 + 单键增量；tree 折叠预览 + 点击展开（DOM 惰性物化） | 嵌套模型渲染；展开/折叠零 Tcl 往返 |
| `ui::chart -vegalite` | ✅ v1 | Vega-Lite 子集（line/bar/point/area，x 序数等距）；spec 过桥、数据走 model 通道（spec.data.name） | 500 点流式 16ms；rAF 合并重绘 0.25ms/帧；组件/spec 不重建 |
| `ui::trace` | ✅ v1 | span 树甘特图；折叠不改对齐、点条缩放/复位全客户端 | 6 行瀑布全交互零 Tcl 回灌 |
| `ui::confirm` | ✅ v1 | 模态动作走 -command 回调（绑 ok；cancel 经 ui::on），决定后禁用、configure 重武装 | 创建脚本跑到底；决定异步回灌（不阻塞） |
| `ui::form` | ✅ v1 | JSON Schema 子集（string/integer/number/boolean/enum + required）；值走模型 `-path` 基路径；客户端校验挡提交 | 击键零回灌、合法提交才回灌 commit（不阻塞） |
| `ui::tabs` | 缺失 | surface 内组织 | 切换保各页状态 |
| markdown `-text` | 缺失 | 渲染需 sanitize | XSS 用例全拒 |
| L2 组件（popover/splitter） | 冻结 | 等真实拉动 | — |

## 3. L2 数据通道与查询（Sentry/ELK 的核心，当前空白）

| 工作块 | 状态 | 说明 |
|---|---|---|
| **HTTP 摄取/客户端** | 缺失 | 双形态：native 宿主提供 server 端点（事件写入）；wasm 侧 `http` 命令包 fetch（拉取+webhook 上报）。事件格式 JSON Lines |
| **事件存储** | 缺失 | 选型待定：native = SQLite（FTS5 全文索引覆盖日志搜索大半）；wasm = IndexedDB 或内存 ring buffer。rtcl-data 扩展 `db::*` 词表 |
| **查询管道词表** | 缺失 | `log::search / stats / top / rare / timechart` 等，输出为 Tcl dict/list——直接 `ui::model set` 进组件。与 SPL 对位但保持 Tcl 组合性 |
| **规则/告警引擎** | 缺失 | `after` 轮询 + 规则 proc（条件 → 动作：toast/webhook/新 surface）；规则即脚本，可存档 |
| **大数据二进制通道** | 冻结（P2） | Arrow IPC over buffer_paths 式通道；table 到十万行级再启 |

## 4. L3 应用模式层（配方与骨架）

每类应用 = L1/L2 的组合配方 + 一个 `app/*.tcl` 骨架：

| 应用 | MVP 切片（最小可演示闭环） |
|---|---|
| Sentry-like | ingest 端点收错误事件 → fingerprint 聚合 → `ui::table` issue 列表 → 行点击开详情 surface（kv+tree+trace）→ 规则 proc 计数告警 |
| ELK-like | shell 输入 `log::search` → `ui::chart` 时间直方图 + `ui::log` 结果流 → 点击直方图桶缩窄时间窗（模型联动） |
| Jupyter-like | shell 历史即 cell 文档；`ui::surface` 为富输出锚点；脚本存档/重放（op 日志天然支持）；widget 交互已有 |
| 共性模式库 | 钻取（行事件→新 surface）、联动筛选（chart 点击→model 过滤→table 刷新）、告警三件套（规则→toast→详情链接） |

## 5. L4 交付与生态

| 工作块 | 说明 |
|---|---|
| 单页静态打包 | wasm+js+html 一目录，离线可跑（build.sh 已半就位） |
| 脚本存档/分享 | dashboard = 可重放 Tcl 脚本；git 可 diff；`source` 即部署 |
| LLM 回路 | `ui::catalog` → prompt 词表；safe interp 执行；错误结构化回执（契约 §5/§6）；**流式渲染免费拥有**——LLM 逐行产出 Tcl 命令即逐条渲染，无需专用增量解析器（对照 docs/UI-GENUI.md）；修改走 append-only delta + `ui::snapshot` 折叠基线（契约 §2.6） |
| 主题系统 | CSS 变量已起步；主题包 = 一段 CSS 文件；词表零感知 |

## 6. UI 设计器问题：做检视器，不做设计器

**结论：不提供拖拽设计器；提供 catalog 驱动的检视器（inspector）。**

理由（展开见 UI-TK-COMPAT.md §3）：

1. 目标用户（开发者/运维/LLM）表达 UI 意图的最高效路径是
   **命令/脚本**，不是拖拽；dashboard 的可重放脚本形态与拖拽
   设计器的二进制工程文件形态根本冲突。
2. 拖拽设计器开发成本高（画布/对齐/undo），收益场景窄。
   （修正记录：visualtkinter.com 现行版已证明"画布拖拽 + 网格
   轨道推断"可行——grid 模式生成 row/column/columnspan/sticky +
   `columnconfigure weight`。这不改变上述结论：它解决的是"设计器
   能否产出结构化布局"，而我们的判断是"目标用户是否需要设计器"。
   若将来需要，轨道推断交互已被验证可参考，见 UI-TK-COMPAT.md §3.1。）
3. 但**探索性调参**（改文案/尺寸/variant 所见即所得）是真实需求，
   检视器覆盖它且几乎免费——catalog spec 就是表单 schema
   （enum→下拉、bool→勾选、size→单位输入、script→代码框）。

检视器工作块（L1 组件稳定后启动，自举 demo）：

- 开发模式开关：hover 高亮 + 点选组件（JS 侧 overlay，不进 op 协议）
- 属性面板：`ui::catalog <type>` 自动生成表单——**用 ui::* 渲染
  ui::* 的检查器**（自举验证场）
- 修改 → `ui::configure` op（双向机制的现成演示）
- **copy as command**：把选中组件的创建命令回显进 shell——
  学习工具 + 脚本化起点（layout 的 fill/weight/gap 也在此表单化）
- 可选增强（后置）：拖 divider 改 `-weight` 数值——布局机制
  已为此备好语义，无需画布

## 7. 关键路径与并行线

```
关键路径：L1 table/log/chart → L2 存储+查询词表 → L3 配方 demo
并行线 A：Worker 形态（不阻塞 L1，但 L3 演示前应具备）
并行线 B：safe interp + LLM 回路（可在 L3 骨架期并行）
检视器：L1 稳定后，与 L3 并行
```

建议下一块：**L2 数据通道**（ROADMAP §3——查询/SQL 面板、
Worker 形态；P0/P1 组件层至此全齐）。
