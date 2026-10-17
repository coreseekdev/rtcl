# 生成式 UI 生态调研：OpenUI / json-render / A2UI × rtcl ui::*

状态：调研纪要（2026-10-17）。结论用于校准契约（UI-DSL.md）与
LLM 工作流设计（ROADMAP.md L4）。

## 1. OpenUI（Thesys，[thesysdev/openui](https://github.com/thesysdev/openui)）

定位：**生成式 UI 的开放标准**——LLM 输出不再是纯文本，而是流式
生成结构化 UI。流水线：

```
Component Library (Zod schemas) → System Prompt 生成 → LLM
    → OpenUI Lang 流 → Renderer → Live UI
```

要点：

- **OpenUI Lang**：紧凑行式语言（每行一个组件，
  `root = Stack([header, chart])`），streaming-first——tokens 到达即
  渐进渲染。**不是 JSON**。[benchmark](https://github.com/thesysdev/openui)
  （46 界面，GPT-5 tokenizer）：比 Vercel json-render 省 52.8%，
  比 Thesys C1 JSON 省 51.7%，最高单场景 -67%。
- **组件契约 = Zod schema**，system prompt 由库自动生成
  （`openuiLibrary.prompt()`）。
- **多渲染目标**：React/Vue/Svelte/Angular/React Native/email。
- **Gateway**：生产侧 repair——流式输出在到达用户前修复无效片段；
  多 provider failover。
- **生态集成**：LangChain/LangGraph/Vercel AI SDK/AG-UI；
  `@openuidev/a2ui` 桥接 Google A2UI v1.0 协议；
  `setup-agent` 把用法写入 AGENTS.md 供 coding agent 使用；
  devtools 包含 Inspect widget（检视器先例）。
- 背后公司 Thesys（类 Vercel/Next.js 关系）；开源。

## 2. 与 rtcl ui::* 的同构对照

| OpenUI | rtcl ui::* | 评估 |
|---|---|---|
| Component Library + Zod schema | catalog spec（ui::define，kind/meta/default） | 同构 ✅ |
| prompt 由库生成 | `ui::catalog` 内省即词表 | 同构 ✅（我们少一层模板代码） |
| OpenUI Lang（行式、流式） | **Tcl 命令流** | 同构且更强：Tcl 图灵完备，条件/循环/数据加工无需出离语言；解释器逐条执行即发 op，**天然流式渲染，无需专用流式解析器** |
| token 效率（vs JSON -52%） | Tcl 命令同等紧凑（`ui::button .s.b -text Hi` ≈ 一行） | 同级 ✅ |
| Gateway repair（修复无效输出） | catalog 严格校验 + Tcl 错误回执 | 哲学分歧：repair 优化 C 端体验；严格校验优化 **agent 学习回路**（错误反馈驱动修正），且我们的 op 原子性保证 UI 不落破碎态。建议保持校验路线，repair 作为 LLM 侧的提示词策略补充 |
| 多渲染目标 | 渲染器注册表（DOM 现行；Taffy/desktop 预留） | 同构 ✅ |
| Inspect devtools widget | 检视器（ROADMAP §6，catalog 驱动） | 先例验证 ✅ |
| A2UI 协议桥 | — | A2UI 是 Google 主导的协议层标准；我们的 op 协议是私有线格式。**若生态聚合，A2UI 兼容层可作为渲染器侧的适配器**（op → A2UI surface model），非当下优先级 |

## 3. 对契约的校准结论

1. **"Tcl 词表 vs JSON 线格式"的决策被第三方商业验证**——行式
   紧凑语言是生成式 UI 的正确形态（token、流式、可读性三赢）。
2. **流式渲染我们免费拥有**：LLM 生成场景 = 逐行把 Tcl 命令喂进
   解释器（`eval` 每行即渲染），无需 OpenUI 那样的增量解析器。
   需在 ROADMAP L4 明确此用法并做 demo。
3. **校验 vs repair**：保持 catalog 严格校验 + 结构化错误回执
   （agent 回路），不修复杂片；UI 原子性已由 op 批次保证。
4. **修改语义**是 OpenUI 未回答而我们必须回答的问题（LLM 反复
   调整 UI 时如何避免全量重生成、丢失视图状态）——见 UI-DSL.md
   §2.6 的 append-only + snapshot 纪律（本次契约补充）。
