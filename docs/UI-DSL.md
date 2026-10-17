# rtcl Web UI DSL（ui::*）契约 v1



状态：草案（开发中）。本文件冻结 web shell 右侧面板的 UI DSL 契约，
是 Tcl 词表层、Rust 桥接层、JS 运行时三方的共同依据。

## 1. 架构总览

```
Worker 线程                        主线程
┌──────────────────┐  postMessage  ┌───────────────────────────┐
│ rtcl (wasm)      │  op[] (JSON)  │ ui-runtime                │
│  ui::* 词表/catalog│ ───────────→ │  op 解释器                 │
│  ui::model 树    │               │  widget 存储(path→record)  │
│  └→ _ui (唯一桥) │ ←──────────── │  模型存储 + 订阅           │
│  post_event ←─── │ event (JSON)  │  渲染器注册表(type→impl)   │
└──────────────────┘               └───────────────────────────┘
```

三条纪律：

1. **Worker 里没有 DOM**：Tcl→JS 的调用同步但 fire-and-forget；
   禁止"问 DOM"的同步查询（几何、滚动位置等走异步 `ui::query` + 事件回灌）。
2. **结构与数据分离**：建结构用 `ui::view::*`/词表命令，喂数据用
   `ui::model set`；规则/LLM 层刷新数据时不得触碰结构。
3. **Tcl 侧持有语义**：参数校验、catalog 内省、批量化都在 Tcl 完成；
   JS 侧只做防御性 sanity check。

## 2. 命令词表 v1

### 2.1 结构与视图（view 域）

```
ui::surface .s ?-title t? ?-catalog c?     创建面板（≈ createSurface）
ui::row|column|card|tabs|divider 路径 ?选项...?
ui::text 路径 ?-text t? ?-variant v?       -text 支持 markdown
ui::image 路径 -src url | ui::icon 路径 -name n
ui::button 路径 ?-text t? ?-variant v? ?-command script?
ui::field 路径 ?-label l? ?-variable var? ?-path /p?
ui::checkbox|slider|choice|datetime 路径 ?选项...?
ui::configure 路径 ?选项...?               局部更新，保状态
ui::destroy 路径                           删除 widget/surface
ui::batch { ... }                          构建事务：op 攒批一次过桥
```

路径即 Tk 风格 widget 路径（`.s.b` 的父路径 `.s` 必须已存在，
JS 侧严格校验——报错帮助暴露生成层 bug）。

### 2.2 数据模型（model 域）

```
ui::model set /pods $data                  Tcl→JS：写模型，绑定者重渲染
ui::model get /pods                        读 Tcl 侧模型树
ui::model unset /pods/0
ui::model sync                             把模型树快照推给 JS（批量）
```

模型树在 Tcl 侧是 dict；`/`-分隔路径。模型更新天然保视图状态
（结构不动，输入焦点/滚动位置不丢）。

输入组件的双向契约（借 A2UI 读写契约）：

- **View→Model（本地）**：击键只写 JS 侧本地模型，不过桥；
- **提交（action）**：`-command` 触发时，`-path` 声明的切片回写为
  Tcl `set`（进 transcript），再执行回调脚本。

### 2.3 分子组件（L1，运维粒度）

```
ui::table  路径 -path /pods -columns {...} ?-rowactions {...}?
ui::chart  路径 -vegalite $spec            payload 即 Vega-Lite spec
ui::log    路径 -path /deploylog           流式追加
ui::kv     路径 -path /info                键值展示
ui::tree   路径 -path /obj                 JSON 树
ui::diff   路径 -old $a -new $b
ui::confirm 路径 -danger ?-preview $d? -command {...}
ui::form   路径 -schema $jsonschema -command {...}
```

### 2.4 事件与动作

```
-command script        事件回调（脚本，事件触发时 eval）
ui::on 路径 事件 {动作}  声明式动作表（dialog/toast/reload/...），
                        是 -command 的结构化糖，供规则层生成
```

事件回灌（JS→Tcl）经 `Interp::post_event` 进入事件队列，
在 Worker 空闲时立即泵入执行：

```json
{"event":"action","path":".s.b","name":"click"}
{"event":"commit","vars":{"/form/email":"x@y"}}
{"event":"bgerror","path":".s.t","message":"renderer threw"}
{"event":"query-result","id":7,"value":...}
```

### 2.5 catalog 与扩展

```tcl
ui::define Button {
    -text     {string {}}
    -command  {script {}}
    -variant  {enum {primary borderless} {}}
}
```

`ui::define` 由规格生成词表命令：校验 args → 规范化 record →
追加 op。catalog 是 Tcl 侧单一事实源：

- `ui::catalog` —— 内省全部词条；`ui::catalog Button` —— 单词条
- 喂 LLM 的 prompt 从 catalog **生成**，与实现不漂移
- safe interp 策略：可按词条裁剪（如只放行 `ui::model` 域）

### 2.6 协作纪律：append-only 修改与快照折叠

- 所有 UI 变更以**追加**形式发生：改属性 = 追加 `ui::configure`；
  删除 = 追加 `ui::destroy`（墓碑）。协作历史（transcript）
  不被改写。多生成源（人 / LLM / 规则 proc / 事件回调）共享同一
  追加流：无锁无争，顺序即历史。
- **快照折叠**：`ui::snapshot path ?-model /prefix?` 从注册表
  反生成等价构建脚本（create 命令序列，含 props 与 -command；
  `-model` 附带导出模型切片为 `ui::model set` 行）。
  性质：snapshot 是纯函数（当前状态 → 脚本）；
  snapshot + 后续 delta ≡ 完整历史。
- 用途：分享/部署（新会话 `source` 即重建）；日志压缩（折叠点
  之前可弃）；**LLM 上下文管理**——长 delta 流折叠为基线，
  生成者只见基线 + 其后 delta，避免全量重生成的视图状态丢失
  与 token 浪费（生成式 UI 生态对照见 docs/UI-GENUI.md）。
- 语义损失声明（v1）：bind 统一重建为 `-path`（`-variable` 糖
  不还原）；同一事件的多个回调脚本，首个并入创建行 `-command`，
  其余重建为 `ui::on` 行。

## 3. op 协议（桥线格式）

`_ui` 是唯一注册到 JS 的命令，载荷是 JSON 的 op 数组：

```json
[{"op":"create","path":".s.b","type":"Button","props":{"text":"Hi"}},
 {"op":"set","path":".s.b","props":{"text":"Done"}},
 {"op":"delete","path":".s.b"},
 {"op":"model","sub":"set","path":"/pods","value":[]},
 {"op":"model","sub":"unset","path":"/pods/0"},
 {"op":"query","id":7,"path":".s.l","what":"scrollpos"}]
```

- 一次 `_ui` 调用携带一个 op 数组；`ui::batch` 外每条词表命令
  自成一个单元素批次。
- `query` 的结果经 `query-result` 事件异步回灌（配合 `vwait`）。
- op 协议是平台中立的唯一保留点：渲染器可以 DOM 耦合，
  将来换 desktop 渲染器不改协议。

## 4. 浏览器纪律（必须遵守）

1. **脚本跑到底**：Tcl 脚本不得长时间运行；长活交给 JS（fetch/
   轮询/流式），完成事件回灌。
2. **`vwait` 只等内部事件**：Worker 里 `postMessage` 只在 Worker
   空闲时投递，`vwait` 自旋会饿死外部事件投递 → **禁止 vwait
   等待 DOM 事件**（`after` 定时器可以）。`ui::confirm` 等模态
   交互必须用 `-command` 回调形式，而不是 vwait 等返回值。
   （v2 候选：SharedArrayBuffer + `Atomics.wait` 可实现真阻塞，
   需 cross-origin isolation，暂不投入。）
3. **不问 DOM**：布局交给 CSS；widget 状态以模型为准。

## 5. safe interp 策略

- AI 生成的代码在 child interp 中执行（`interp` 命令已有）；
- UI 权限按域授予：默认只放行 `ui::model` 与只读内省；
  结构创建需显式授权；
- `set_max_steps`（已有）限制生成代码的执行步数预算。

## 6. 错误回路

- Tcl 侧校验错误：词表命令直接抛 Tcl 错误，errorInfo 指向用户
  输入行（REPL 红字，LLM 场景包装为结构化回执）；
- JS 侧错误（未知 type、父路径缺失、renderer 抛异常）：
  `bgerror` 事件回灌，shell 显示。

## 7. 里程碑

- M1 Rust 地基：HostClock 落地（修 event.rs wasm panic）、
  `post_event`、wasm `dispatch_event` ✅
- M2 Tcl catalog 词表层（本文件 §2 的 L0 部分，
  `crates/rtcl-core/src/ui.tcl` + `tests/ui_tcl.rs`）✅
- M3 JS 运行时（`crates/rtcl-wasm/www/ui-runtime.js`：op 解释器/
  widget+模型存储/渲染器注册表 + L0 纯 DOM 渲染器）✅
- M4 demo：`crates/rtcl-wasm/www/ui.html`（左 shell 右 UI，
  counter + form 端到端验证通过；`crates/rtcl-wasm/build.sh` 后
  在 crate 根起 http server 访问 `/www/ui.html`）✅
- M5 候选：Worker 形态（`after`/`vwait` 纪律在 Worker 下才完整
  生效）、`query` op、L1 分子组件（table/chart/log）、markdown
  text、Base UI/TanStack 替换纯 DOM 渲染器
- M5 ✅ `ui::table` v1（L1 首个分子组件，2025-10-17）：词表
  （Table define：`-path` bind、`-columns` list kind、
  `-rowactions` script）+ JS 渲染器（虚拟滚动 spacer/池 + 行级
  增量复用节点 + 事件委托）+ demo/验收页（M1 明细见
  crates/rtcl-wasm/www/ui-table-acc.html，落地中的契约缺口收进
  本文件 §8）

## 8. 变更建议（v1 落地中发现的缺口；本节只记录，未改协议）

以下条目在 `ui::table`（M5 首个 L1 分子组件，已落地）实现过程中
踩实，属于**契约缺口候选**，等下一轮契约评审定夺再进 §2 正文：

1. **事件行级 detail**：`::ui::fire 路径 事件 ?detail?`（第三参
   可选，目前仅行场景使用；action 脚本里以 `$ui_row` 读行
   号——Tk `%` 替换的轻量替身，回调结束后
   `unset -nocomplain ui_row` 复位）。若事件未来改走 JSON 形状
   （§2.4 的 JSON 行方案），建议该形状增加 `detail` 字段，别把
   行号塞进事件名。
2. **bind 订阅前缀扇出**：Table 订阅 `/pods` 时，`ui::model set
   /pods/42 …` 也要送达（回调第二参 = 实际变更路径；第一参 =
   绑定前缀处的整值）。Field/Checkbox 只消费第一参，行为不变。
   建议 §2.2 正文补一句"订阅按前缀匹配，回调第二参携带变更
   路径"。
3. **cell 级模型增量需行值为 dict**：`ui::model set /pods/42/status
   X` 下钻标量行报 `missing value to go with key`（dict set 语义，
   tclsh 8.6 同源，非 bug）。Table v1 不依赖 cell 级：行值整体
   替换；若要 cell 级，行必须存 dict（契约 §2.2 未写这层前提）。
4. **`ui::model sync`**（§2.2 已列）：ui.tcl 词表未实现，快照走
   `ui::snapshot` 反生成脚本路线（§2.6）。落地时建议明确它与
   snapshot 的分工：sync = 一次性全量推桥（初始挂载），snapshot
   = 反生成等价构建脚本（协作折叠）。

实现里程碑取证（Ark 旁注，非契约）：喂数吞吐依赖嵌套 `dict set`
的就地快路径（rtcl 解释器侧，2025-10-17 起 `feedpods 100000` ≈
2.4s，rtcl-cli 基准见 commit 3101690）；喂数脚本里 `format` 的
参数必须与命令同物理行（`[...]` 内完整命令后的换行即命令终结
符，tclsh 同源）。
