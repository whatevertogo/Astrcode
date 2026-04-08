<!--
Sync Impact Report
- Version change: 1.1.0 -> 1.2.0
- Modified principles:
  - Architecture Constraints: 双向依赖约束收紧，runtime 门面量化约束
  - Governance: 新增合规审计要求
- Added sections:
  - VI. Runtime Robustness（运行时健壮性）
  - VII. Observability & Error Visibility（可观测性与错误可见性）
- Removed sections:
  - None
- Templates requiring updates:
  - ✅ D:\GitObjectsOwn\Astrcode\.specify\templates\plan-template.md (Constitution Check 新增 VI/VII 检查项)
  - ✅ D:\GitObjectsOwn\Astrcode\.specify\templates\tasks-template.md (Constitution-Driven Task Rules 新增健壮性和可观测性任务)
  - ✅ D:\GitObjectsOwn\Astrcode\.specify\templates\spec-template.md (validated, no change required)
  - ✅ D:\GitObjectsOwn\Astrcode\AGENTS.md (validated, no change required)
  - ✅ D:\GitObjectsOwn\Astrcode\docs\architecture\architecture.md (validated, no change required)
  - ✅ D:\GitObjectsOwn\Astrcode\CLAUDE.md (validated, no change required)
- Follow-up TODOs:
  - plan-template.md Constitution Check 需补充 VI/VII 对应检查项
  - tasks-template.md Constitution-Driven Task Rules 需补充健壮性和可观测性相关任务模板
-->
# Astrcode Constitution

所有文档都需要用中文来生成和书写,方便用户观看

当前文件夹的MEMORY.md 内有更多后续迭代宪章细节

## Core Principles

### I. Durable Truth First
所有可回放、可解释、可审计的历史行为 MUST 以 durable 事件为唯一事实源。`StorageEvent`
及其持久化日志定义历史真相；live registry、cache、recent tail、render model、
SSE 客户端状态只允许做运行态补充或读取加速，MUST NOT 成为解释过去行为所必需的真相层。
任何涉及 replay、compaction、subrun 重建、child session 导航或范围过滤的能力，MUST
先从 durable 事实推导，再按需叠加 live 运行态。理由：一旦历史解释依赖内存状态，上层就会
不可避免地产生多套真相。

### II. One Boundary, One Owner
每个边界 MUST 拥有且只拥有一类核心职责，并对该职责承担最终解释权。`runtime-session`
拥有 session 真相与 durable 生命周期；`runtime-execution`
拥有执行编排；`runtime-agent-loop` 拥有单次 turn 的模型与工具循环；
`runtime-agent-control` 拥有 live 子执行控制；`server` 只拥有传输投影；
`frontend` 只拥有渲染归并。任何边界都 MUST NOT 通过环形调用把自己声称拥有的职责再委托回上层，
也 MUST NOT 长期保留语义重叠的双轨 façade。理由：职责重叠会直接演化成 god object、
回调环和无法预测 blast radius 的重构。

### III. Protocol Purity, Projection Fidelity
`protocol` MUST 保持 DTO-only，不得承载运行时策略、状态机、默认推断或 UI 归并逻辑。
`protocol` 中的 struct/enum 只允许包含字段定义和派生自 `#[derive]` 的标准 trait 实现
（Debug、Clone、Serialize、Deserialize 等）；自定义 `impl` 块只允许包含构造辅助
（如 `new()`）和纯数据访问方法，MUST NOT 包含业务规则判断或跨类型编排逻辑。
`server` MUST 作为纯投影层，把 runtime 事实映射为稳定协议；`/history` 与 `/events`
MUST 暴露相同 envelope 语义。后端 MUST 用显式协议表达 tool call、subrun 生命周期、
错误和 compaction 边界，而不是把这些语义留给前端通过工具名、事件顺序或渲染启发式推断。
理由：一旦传输语义偏离运行时事实，server 和 frontend 就会同时维护一套额外业务规则。

### IV. Ownership Over Storage Mode
任务 ownership、取消链路、父子执行身份和触发来源 MUST 显式建模，MUST NOT 从事件写入位置、
session mode 或 UI 视图反推。`SharedSession` 与 `IndependentSession`
只允许改变"事件写到哪里"，MUST NOT 改变"谁拥有执行、谁可以控制执行、父子关系如何解释"。
任何由工具触发的子执行都 MUST 保留其与触发 tool call 之间的稳定关联。理由：storage mode
是实现细节，ownership 才是领域事实。

### V. Explicit Migrations, Verifiable Refactors
项目允许破坏性重构，但每次涉及 durable 事件、协议契约、公共 runtime surface、依赖方向或边界所有权的变更，
MUST 附带显式 caller inventory、迁移顺序、兼容策略和验证命令。重大架构文档 MUST
把已确认事实、问题诊断、设计提案分离，禁止在同一文档中混写。保留兼容性不是默认义务；如需兼容，
必须明确说明为什么兼容成本值得承担。任何完成声明都 MUST 包含与改动范围匹配的 Rust 与前端验证证据。
理由：项目追求干净架构，不追求"看起来不破坏"的模糊过渡。

### VI. Runtime Robustness
生产代码 MUST NOT 包含可能 panic 的路径。所有锁获取（`Mutex::lock`、`RwLock::read`/`write`）
MUST 使用恢复机制（如 `with_lock_recovery` 或返回 `Result`），MUST NOT 直接调用 `.unwrap()`
或 `.expect()`。所有数组/切片索引访问 MUST 使用安全变体（`.get()`、`.get_mut()`）或前置
长度断言。所有 `tokio::spawn` 创建的异步任务 MUST 保存 `JoinHandle` 并具备明确的取消机制
（`JoinHandle::abort` 或 `CancellationToken`），MUST NOT 以 fire-and-forget 方式丢弃
任务句柄。持有 `Mutex`/`RwLock` 守卫时 MUST NOT 跨越 `.await` 点；如需在锁内执行异步操作，
MUST 先释放锁再 await，或使用专门的 actor/channel 模式解耦。
理由：panic 会终止进程，fire-and-forget 会泄漏资源，持锁 await 会死锁——这三种模式在
生产环境中都是不可接受的。

### VII. Observability & Error Visibility
关键业务操作（会话创建与销毁、工具执行开始与结束、子代理调度与完成、配置热重载）MUST
有结构化日志记录。日志级别 MUST 语义一致：操作失败 MUST 用 `error!`，降级或重试 MUST
用 `warn!`，正常关键路径 MUST 用 `info!`，调试细节 MUST 用 `debug!`。MUST NOT 在
生产代码中使用 `println!` 或 `eprintln!` 替代日志。
错误 MUST NOT 被静默忽略：`.ok()`、`let _ =` 只允许在紧邻的行内注释显式说明忽略原因时使用。
`map_err` 转换 MUST 保留原始错误信息（通过 `#[source]` 或 `error.to_string()`），
MUST NOT 使用 `map_err(|_| ...)` 丢弃原始错误上下文。
理由：无法观测的系统无法诊断；静默吞掉的错误会变成不可复现的幽灵 bug。

## Architecture Constraints

- `protocol` 与 `core` 之间 MUST NOT 存在任何直接依赖。如果两者需要共享类型，该类型
  MUST 定义在 `protocol` 中（纯 DTO）并通过 mapper 在 `core` 侧转换；`core` 的
  领域类型 MUST NOT 出现在 `protocol` 中。跨边界数据一律通过显式 DTO + mapper 传递。
- `core` 定义稳定契约与共享领域类型；实现层 MUST 通过这些契约协作，而不是横向偷依赖。
- `storage` 只实现持久化，不得承载执行编排、查询语义或 UI 专用逻辑。
- `runtime-...-loader` 系列 MUST 依赖 `core` 而非 `runtime`，避免装配层反向渗透到加载层。
- `runtime-prompt`、`runtime-llm`、`runtime-config`、`runtime-registry` 等独立子系统 MUST
  保持编译隔离；`runtime` 只做组合，不复制子 crate 逻辑。
- `runtime` 门面 MAY 提供统一入口，但 MUST NOT 成为第二套业务实现层；一旦某项逻辑已经下沉到子边界，
  门面只允许装配和转发。`runtime` 门面下的单个文件 MUST NOT 超过 800 行；超过时 MUST 拆分为
  独立子模块或下沉到对应的子边界 crate。`RuntimeService` MUST NOT 直接持有业务状态
  （如 session map），业务状态 MUST 由对应的子边界（如 `runtime-session`）持有和管理。
- `frontend` 只能消费后端稳定协议并做 render aggregation，MUST NOT 反向定义后端领域模型。
- `session tree`、`subrun view`、`child navigation` 等可视结构只能是 read model，MUST NOT
  反向固化为核心领域对象。

## Development Workflow & Review Discipline

- 任何重大架构、协议、持久化或边界调整 MUST 先形成规格或设计文档，再进入实现。
- 当变更满足以下任一条件时，设计文档 MUST 强制拆分为 findings、design、migration 三层：
  - 涉及 durable 事件格式或字段变更。
  - 涉及公共 runtime surface 的增删改。
  - 跨边界依赖方向发生变化。
  - 删除或替换任何已有外部调用方的模块或接口。
- 不满足上述触发条件的变更 MAY 使用更轻量的设计文档，但仍必须把事实观察与设计结论分开表达。
- 评审时 MUST 明确回答七个问题：durable 真相在哪里、边界 owner 是谁、协议如何投影、
  ownership 是否独立于 storage mode、旧调用方和旧数据如何迁移、是否存在 panic 路径或
  持锁 await、关键操作的日志和错误暴露是否充分。
- 删除或移动公共入口前，MUST 先列出现有调用方、替代入口和删除前提；依赖"编译报错后再修"不构成迁移计划。
- 代码注释如有必要，MUST 使用中文，且解释为什么与做了什么，而不是复述代码字面含义。
- 每次改动完成前，MUST 运行与改动范围匹配的验证。仓库主线验证基准为：
  `cargo fmt --all -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、
  `cargo test --workspace --exclude astrcode`；若涉及前端，还 MUST 补充 `cd frontend && npm run typecheck`
  及相关前端检查。

## Governance

本宪法高于项目内其他架构说明、模板默认文本和临时流程约定。若任何文档、模板或说明与本宪法冲突，
冲突方 MUST 在同一变更中被修订，而不是依赖口头解释维持一致性。

原则之间的关系是依赖而非优先级：I 是 III 和 IV 的前提，II 是 V 和 VI 的前提，III、IV、V
是 I 和 II 的落地验证手段，VI 是 II 在运行时的刚性保障，VII 是所有原则的可诊断性基础。
当原则之间表面冲突时，MUST 先检查是否存在违反 I 或 II 的隐含前提，而不是直接裁定孰优孰劣。

本宪法的修订 MUST 满足以下要求：

- 修订者 MUST 在文件顶部更新 Sync Impact Report，并列出受影响原则、章节、模板与后续事项。
- 修订者 MUST 检查 `.specify/templates/plan-template.md`、
  `.specify/templates/spec-template.md`、`.specify/templates/tasks-template.md`
  以及相关指导文档，确保原则变化已经同步或明确说明无需同步。
- 版本号 MUST 使用语义化规则：
  - MAJOR：删除原则、重新定义原则含义、或改变治理要求导致既有流程失效。
  - MINOR：新增原则、章节，或对既有要求做实质性扩展。
  - PATCH：仅做措辞澄清、排版、错字修复或不改变义务的说明。
- 每次 plan、design review、PR review 都 MUST 检查是否触及本宪法原则。
- 未通过宪法检查的计划或实现不得视为完成，即使代码已经编译通过。
- 仓库 MUST 维护 `CODE_REVIEW_ISSUES.md` 文件，记录所有已知的宪法偏离项。
  每个偏离项 MUST 包含：违反的原则、涉及的文件和行号、当前状态（待修复/修复中）、
  修复优先级。该文件 MUST 在每个里程碑或重大合并前进行合规审计，确保偏离项数量只减不增。

**Version**: 1.2.0 | **Ratified**: 2026-04-07 | **Last Amended**: 2026-04-08
