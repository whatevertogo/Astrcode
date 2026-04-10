# Research: 删除死代码与兼容层收口

## Decision 1: 用“真实消费者 + 明确 owner”定义支持面

**Decision**  
本次清理以“当前产品流程是否真的调用它、是否有人能说清它归谁负责”为唯一标准来决定保留、迁移或删除。

**Rationale**  
代码库里已经出现多类“看起来存在、实际上没人用”的 surface：有的只有测试引用，有的只有文档宣传，有的只有骨架实现。这些东西最大的风险不是暂时没用，而是会持续误导后续设计和 review。

**Alternatives considered**

- 按“已经实现了就先保留”处理：会让骨架接口和预实现无限期存在。
- 按“测试覆盖到就算有用”处理：会把测试自证变成保留理由。
- 按“将来可能有 operator/外部调用方”处理：没有 owner 的猜测不足以构成支持面。

## Decision 2: 删除无人消费的 parent-child summary projection，只保留真实摘要事实

**Decision**  
删除 `loadParentChildSummaryList`、`loadChildSessionView`、`buildParentSummaryProjection` 及对应 server route；继续保留 `SubRunHandoff.summary` 与 `ChildSessionNotification.summary` 这两类真实被消费的摘要事实。

**Rationale**  
当前 UI 已经通过现有消息流、child notification 和直接打开子会话来完成浏览，不需要额外的 parent-summary API 或 projection。真正有价值的是摘要事实本身，而不是一层没有消费者的重复投影。

**Alternatives considered**

- 保留这些 projection，等以后 UI 再接：这是典型预实现。
- 删除所有 `summary`：会误删当前真实在用的 handoff/notification 摘要。
- 只删前端不删后端：会留下 server 孤儿 surface 和维护成本。

## Decision 3: `cancelSubRun` 必须先迁移到 `closeAgent`

**Decision**  
`cancelSubRun` 不是立即删除项；先把当前 UI 取消动作迁移到 `closeAgent` 协作能力，再删除 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel` 及其前端包装。

**Rationale**  
当前 `Chat -> SubRunBlock -> useAgent -> cancelSubRun` 仍是活跃主线流程。直接删 route 会让“取消子会话”功能消失，和本次特性的“删死代码，不伤主线”目标冲突。

**Alternatives considered**

- 直接删除 cancel route：会打断活跃功能。
- 永久同时保留 cancel route 和 `closeAgent`：会制造双轨主线。
- 在 UI 外部做一次临时桥接：复杂度更高，而且会再引入一层兼容。

## Decision 4: 删除无人消费的 public HTTP surface，不保留“未来入口”

**Decision**  
删除 `/api/v1/agents`、`/api/v1/tools`、`/api/runtime/plugins`、`/api/runtime/plugins/reload`、`/api/config/reload` 这类当前没有产品入口、没有明确 owner、只剩实现和测试自证的 public surface。

**Rationale**  
这些接口的共同问题不是“实现不完整”，而是没有当前消费者和产品语义。继续把它们暴露在 server/docs 里，只会让仓库持续暗示一个不存在的 operator/API 面。

**Alternatives considered**

- 保留但标注 experimental：没有消费者的 surface 不是因为标签就会变得合理。
- 把它们留到未来真的需要时再决定：现在不删，未来只会更难删。
- 只删除 execute 类骨架，保留 list/status 类：如果整体没有 owner，局部保留也只是半截兼容。

## Decision 5: legacy 历史改为明确失败，不再对外公开 downgrade 语义

**Decision**  
旧共享历史、descriptor 缺失的 legacy subrun 和 related downgrade 语义不再以 `legacyDurable`、legacy tree、legacy UI 类型等方式对外公开；它们统一收敛为明确失败能力，必要时保留稳定错误码。

**Rationale**  
如果系统已经决定“不支持旧历史作为正常输入”，那最干净的做法是显式失败，而不是继续维持一套“部分可用”的公开状态模型。后者会把兼容逻辑永久固化在 protocol、frontend 和测试基线里。

**Alternatives considered**

- 继续保留 `legacyDurable`：会让旧历史继续占据正式公开语义。
- 完全吞掉错误并当普通空数据处理：会隐藏根因。
- 提供更复杂的升级桥接：本次特性目标是删减而不是增加迁移系统。

## Decision 6: live 文档与 archive 文档分层处理

**Decision**  
更新当前生效的 `docs/spec/*` 与开放项，只保留清理后仍受支持的 surface；archive 文档继续作为历史记录保留，但不再被当前说明引用为现状。

**Rationale**  
如果 live 文档仍宣传已删除或无人消费的接口，代码再干净也会被文档重新污染回来。

**Alternatives considered**

- 一并删除 archive：会丢失历史背景，不必要。
- 什么都不改，只靠实现说话：后续维护者和自动化工具仍会被旧文档误导。

## Decision 7: 测试改为证明“当前主线可用 + 被删 surface 不再存在”

**Decision**  
本次回归测试只保留两类价值：主线流程还可用，已删除 surface 不再暴露。专门证明旧骨架或 legacy projection 仍存在的测试应被删除。

**Rationale**  
测试不应该成为死代码的避难所。只为某个旧 surface 存在感服务的测试，本质上是在帮死代码维持政治生命。

**Alternatives considered**

- 全部保留旧测试以防回归：会把回归目标定义错。
- 只删实现不删测试：会持续制造噪音和维护负担。
