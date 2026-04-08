<!--
Sync Impact Report
- Version change: 1.3.0 -> 1.4.0
- Modified principles:
  - I ~ VII: 移除所有实现模式细节，仅保留架构公理
- Added sections:
  - None
- Removed sections:
  - Architecture Constraints / 工具安全（实现细节）
  - Architecture Constraints / 执行上下文绑定中的具体模式
  - 所有 "How to apply" 实现指导
  - 锁恢复 / 错误链 / 异步句柄等具体 API 引用
- Templates requiring updates:
  - ✅ .specify/templates/plan-template.md (无需变更，Constitution Check 已是原则级)
  - ✅ .specify/templates/tasks-template.md (无需变更)
  - ✅ .specify/templates/spec-template.md (无需变更)
  - ✅ AGENTS.md (无需变更)
- Follow-up TODOs:
  - code_quality_fixes.md 和 tool_security_enhancements.md 已承载具体模式，无需回迁
-->
# Astrcode Constitution

## 核心原则

### I. Durable Truth First

Durable: 事件是所有可回放、可解释、可审计历史行为的唯一事实源。Live 状态、缓存、
渲染模型、客户端状态只允许做运行态补充或读取加速，MUST NOT 成为解释过去行为
所必需的真相层。任何涉及 replay、compaction、子会话导航或范围过滤的能力，
MUST 先从 durable 事实推导，再按需叠加 live 运行态。

### II. One Boundary, One Owner

每个编译/运行时边界 MUST 拥有且只拥有一类核心职责，并对该职责承担最终解释权。
任何边界都 MUST NOT 通过环形调用把自己声称拥有的职责再委托回上层，也 MUST NOT
长期保留语义重叠的双轨 façade。

### III. Protocol Purity, Projection Fidelity

`protocol` MUST 保持 DTO-only，不得承载运行时策略、状态机、默认推断或 UI 归并逻辑。
`server` MUST 作为纯投影层，把 runtime 事实映射为稳定协议；`/history` 与 `/events`
MUST 暴露相同 envelope 语义。后端 MUST 用显式协议表达 tool call、子会话生命周期、
错误和 compaction 边界，而不是把这些语义留给前端推断。

### IV. Ownership Over Storage Mode

任务 ownership、取消链路、父子执行身份和触发来源 MUST 显式建模，MUST NOT 从
事件写入位置、session mode 或 UI 视图反推。Storage mode 只允许改变"事件写到哪里"，
MUST NOT 改变"谁拥有执行、谁可以控制执行、关系如何解释"。

### V. Explicit Migrations, Verifiable Refactors

项目允许破坏性重构，但每次涉及 durable 事件、协议契约、公共 runtime surface、
依赖方向或边界所有权的变更，MUST 附带显式迁移顺序和验证命令。重大架构文档 MUST
把事实观察、设计提案、迁移计划分离。保留兼容性不是默认义务。

### VI. Runtime Robustness

生产代码 MUST NOT 包含可能 panic 的路径。所有并发原语 MUST 具备恢复机制，
MUST NOT 持锁跨 await。所有异步任务 MUST 具备明确的取消机制和生命周期管理，
MUST NOT 以 fire-and-forget 方式丢弃任务句柄。

### VII. Observability

关键业务操作 MUST 有结构化日志记录。日志级别 MUST 语义一致且可区分。
错误 MUST NOT 被静默忽略。错误转换 MUST 保留原始上下文，MUST NOT 丢弃
根因信息。

## 架构约束

### Crate 依赖方向

- `protocol` 与 `core` 之间 MUST NOT 存在任何直接依赖；跨边界走显式 DTO + mapper。
- `core` 定义稳定契约与共享领域类型；实现层 MUST 通过契约协作，不得横向偷依赖。
- `storage` 只实现持久化，不得承载执行编排或 UI 专用逻辑。
- `runtime-...-loader` 系列 MUST 依赖 `core` 而非 `runtime`。
- `runtime-prompt`、`runtime-llm`、`runtime-config`、`runtime-registry` 等 MUST 保持
  编译隔离；`runtime` 只做组合，不复制子 crate 逻辑。
- `runtime` 门面 MUST NOT 成为第二套业务实现层。单个文件 MUST NOT 超过 800 行。
  `RuntimeService` MUST NOT 直接持有业务状态。

### 子会话所有权

- 父子 ownership MUST 以 durable 节点显式记录，MUST NOT 从磁盘路径或 session mode
  推断。缺少显式节点的历史数据 MUST 降级，MUST NOT 伪造关系。
- 父 turn 的正常完成 MUST NOT 自动取消仍在运行的子 agent。
- 父会话 MUST 只消费通知/摘要/终态交付，MUST NOT 混入子会话完整中间事件流。
- 子会话 MUST 可作为独立会话直接打开和查看。
- 关闭传播 MUST 按 agent 层级所有权树执行，MUST NOT 以父 turn 状态为判定条件。

### Agent 协作分层

- 协作工具契约层（模型可调用能力与结果语义）与 runtime 投递层（送达、唤醒、去重）
  MUST 分离，不得混为同一抽象。
- 协作工具输入 MUST NOT 暴露 runtime 内部细节；输出 MUST NOT 返回原始内部结构。
- 协作投递 MUST 具备幂等与去重语义。
- 子 agent 向父 agent 的上行消息 MUST 通过受控协作入口，MUST NOT 直接修改父 agent
  内部状态。

### 前端投影

- `frontend` 只消费后端稳定协议，MUST NOT 反向定义后端领域模型。
- 可视结构（session tree、subrun view、breadcrumb）只能是 read model，MUST NOT
  固化为核心领域对象。
- 父视图 MUST 采用"父侧摘要 + 子侧完整时间线"的双层模型。UI 状态 MUST NOT
  影响 durable 内容的持久化。

### 执行血缘一致性

- 执行血缘索引 MUST 从 durable descriptor 构建，MUST NOT 从事件顺序推断 ancestry。
- 历史回放、增量订阅、范围过滤三条路径 MUST 返回一致结果。
- 缺 descriptor 时 scope 过滤 MUST 失败，不伪造 ancestry。

## 开发纪律

- 重大架构、协议或边界调整 MUST 先形成规格文档再实现。
- 涉及 durable 事件变更、runtime surface 增删、跨边界依赖方向变化、或删除公共入口时，
  文档 MUST 拆分为事实观察、设计、迁移三层。
- 删除或移动公共入口前，MUST 先列出调用方和替代入口。
- 代码注释 MUST 使用中文，解释为什么与做了什么。
- 每次改动 MUST 运行匹配范围的验证：`cargo fmt/clippy/test` + 前端 `typecheck`。
- Plugin 相关类型属于 `core`，不属于 `protocol`。
- 不需要向后兼容；如需兼容必须说明为什么。

## 治理

本宪法高于项目内其他架构说明、模板和临时约定。冲突方 MUST 在同一变更中被修订。

原则之间是依赖关系：I 是 III 和 IV 的前提，II 是 V 和 VI 的前提，III/IV/V 是
I 和 II 的落地验证，VI 是 II 的运行时保障，VII 是所有原则的可诊断性基础。
表面冲突时 MUST 先检查是否违反 I 或 II 的隐含前提。

修订要求：

- MUST 更新文件顶部 Sync Impact Report。
- MUST 检查 `.specify/templates/` 下模板的一致性。
- 版本号 MUST 语义化：MAJOR 删原则或重定义、MINOR 新增原则或实质性扩展、
  PATCH 仅措辞澄清。
- 每次 plan/review/PR MUST 检查是否触及本宪法。
- 未通过宪法检查的实现不得视为完成。

**Version**: 1.4.0 | **Ratified**: 2026-04-07 | **Last Amended**: 2026-04-09
