# Migration: 删除死代码与冗余契约收口

## Migration Principles

- 先做 caller inventory，再删公共入口。
- 先建立 canonical owner，再删重复模型。
- 无消费者 surface 立即删除，不做观察期。
- 活跃主线先迁移，再删 legacy 入口。
- live 文档、测试、夹具与实现必须同批次收口。

## Caller Inventory

| Surface | Current caller state | Migration action |
|--------|-----------------------|------------------|
| `loadParentChildSummaryList` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 client + tests + docs |
| `loadChildSessionView` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 client + tests + docs |
| `buildParentSummaryProjection` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 projection + tests + docs |
| `/api/sessions/{id}/children/summary` | 无当前消费者 | 直接删除 route + DTO + tests + docs |
| `/api/sessions/{id}/children/{child_session_id}/view` | 无当前消费者 | 直接删除 route + DTO + tests + docs |
| `/api/v1/agents*` | 无当前产品入口 | 直接删除 route + protocol + tests + docs |
| `/api/v1/tools*` | 无当前产品入口；execute 为骨架 | 直接删除 route + protocol + tests + docs |
| `/api/runtime/plugins*` | 无当前产品入口 | 直接删除 route + tests + docs |
| `/api/config/reload` | 无当前产品入口 | 直接删除 route + tests + docs |
| `SubRunOutcome` | 与 `AgentStatus` 表达同一状态语义 | 收口到 `AgentStatus` |
| `SubRunDescriptor` / optional `parent_turn_id` | 仅为 downgrade/descriptor 路径续命 | 收口到 `SubRunHandle` 必填字段 |
| `PromptAccepted` / `RootExecutionAccepted` / runtime duplicates | 内部 receipt duplication | 收口到 `ExecutionAccepted` |
| `launch_subagent` on orchestration trait | owner 错位 | 迁移到 `LiveSubRunControlBoundary` |
| `ChildAgentRef.openable` | UI 派生字段混入领域模型 | 删除，由 canonical `child_ref.open_session_id` 判断可打开性 |
| `ChildSessionNotification.open_session_id` | 与 `child_ref.open_session_id` 重复表达同一 open target | 删除外层字段，只保留嵌套 canonical open target |
| child/subrun DTO `status: String` | 协议状态弱类型外泄 | 收口到 protocol 强类型状态枚举 |
| 三层 `PromptMetrics` variant 字段重复 | 同一 payload 在 storage/domain/protocol 三层手工搬运 | 提取共享 `PromptMetricsPayload` |
| 散落的 `Reactive -> CompactTrigger` 手写映射 | compaction 原因归一位置不明确 | 收口到单一映射 owner |
| `cancelSubRun` client + `/subruns/{id}/cancel` route | 当前 UI 仍在调用 | 切到 `closeAgent` 后删除 |
| `legacyDurable` / descriptorless downgrade public semantics | 当前无正向业务价值，只维持 legacy 展示/测试 | 收敛为明确失败，删除 downgrade surface |

## Phase Order

### Phase 1: 锁定支持面与 canonical owner

- 完成 caller inventory
- 在 plan/findings/contracts 中标明 retain / migrate-then-remove / remove
- 确认每个冗余模型的 canonical target

**Exit Criteria**

- 每个候选 surface 都有明确分类
- 每个 duplicated contract 都有唯一 owner
- 没有“先留着看看”的灰色项

### Phase 2: 收口 core/runtime canonical contract

- `SubRunOutcome -> AgentStatus`
- `SubRunDescriptor -> SubRunHandle`
- `parent_turn_id` 改必填
- `PromptAccepted` / `RootExecutionAccepted` / runtime duplicates -> `ExecutionAccepted`
- 为 `AgentEventContext` 增加 `From<&SubRunHandle>`
- `launch_subagent` 迁入 `LiveSubRunControlBoundary`
- `ChildAgentRef` 删除 `openable`，保留 canonical `open_session_id`
- 删除 `ChildSessionNotification` 外层重复 `open_session_id`
- protocol `status: String` -> 强类型状态枚举
- `PromptMetrics` -> 共享 payload
- compaction reason -> durable trigger 映射集中化

**Exit Criteria**

- core/runtime 中不再同时维护两套 subrun 状态、descriptor 或 receipt
- trait owner 清晰
- `openable` 不再存在于 core child ref
- child open target 只保留一份 canonical 字段
- protocol 状态不再是字符串
- prompt metrics 不再维护三份字段清单

### Phase 3: 删除立即可删的 orphan surface

- 删除 parent-child summary clients / projections / server routes
- 删除 `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload`
- 同步删除 protocol DTO、mapper、tests、docs

**Exit Criteria**

- 被标记为“立即删除”的 surface 在代码、测试、docs 中都不再出现
- 当前会话浏览、当前 child session 直开、当前配置读写不受影响

### Phase 4: 切换 child control 与 navigation 主线

- UI cancel 动作切到 `closeAgent`
- 删除 `cancelSubRun` wrapper 与 legacy cancel route
- 前端改用 canonical open target / child session fact，去掉对 duplicated open flag 的依赖

**Exit Criteria**

- 当前“取消子会话”按钮仍可用
- 只剩一条主线 close/cancel 入口
- child session 仍可直接打开

### Phase 5: 删除 legacy downgrade public semantics

- 删除 `legacyDurable` 公开状态
- 删除 descriptor-missing legacy tree / helper / DTO / tests
- 旧输入统一改为明确失败

**Exit Criteria**

- 不再把旧历史包装成“部分可用”
- 前端/协议/运行时不再公开 legacy downgrade surface

### Phase 6: 文档与测试收尾

- 更新 `docs/spec` live 文档
- 删除或改写旧测试/夹具
- 执行完整验证矩阵

**Exit Criteria**

- live 文档只描述保留能力
- tests 只证明保留主线与明确失败

## Checkpoints

### Checkpoint A: Canonical Contract Settled

- `AgentStatus` 成为唯一 subrun 状态词表
- `ExecutionAccepted` 成为唯一内部 receipt
- `SubRunHandle` 成为唯一 lineage owner

### Checkpoint B: Orphan Summary Surface Removed

- 前端不再导出 `loadParentChildSummaryList`、`loadChildSessionView`、`buildParentSummaryProjection`
- server / protocol 不再暴露对应 children summary/view contract
- 保留 `SubRunHandoff.summary` 与 `ChildSessionNotification.summary`

### Checkpoint C: Legacy Cancel Route Removed

- UI 取消动作改用 `closeAgent`
- legacy cancel route 与前端包装被删除
- 主线取消能力通过新入口回归

### Checkpoint D: Skeleton Routes Removed

- `/api/v1/agents*`
- `/api/v1/tools*`
- `/api/runtime/plugins*`
- `/api/config/reload`

以上 route、相关 tests 和 live docs 均已删除。

### Checkpoint E: Legacy Downgrade Surface Removed

- `legacyDurable` 不再出现在公开类型、协议 DTO 或前端状态中
- descriptorless helper 分支和对应测试被删除
- 旧输入改为明确失败

## Validation Matrix

1. `rg -n "SubRunOutcome|SubRunDescriptor|PromptAccepted|RootExecutionAccepted|AgentExecutionAccepted" crates/core crates/runtime crates/runtime-execution crates/runtime-agent-control`
2. `rg -n "loadParentChildSummaryList|loadChildSessionView|buildParentSummaryProjection|ParentChildSummaryListResponseDto|ChildSessionViewResponseDto" frontend crates/protocol crates/server`
3. `rg -n "/api/v1/agents|/api/v1/tools|/api/runtime/plugins|/api/config/reload|subruns/.*/cancel" crates/server frontend docs/spec`
4. `rg -n "legacyDurable|openable|status: String" frontend/src crates/core crates/protocol crates/server`
5. `rg -n "pub open_session_id" crates/core crates/protocol crates/server`
6. `rg -n "PromptMetrics \\{|estimated_tokens: u32,|provider_cache_metrics_supported: bool" crates/core crates/protocol crates/server`
7. `cargo fmt --all --check`
8. `cargo clippy --all-targets --all-features -- -D warnings`
9. `cargo test --workspace --exclude astrcode`
10. `cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`
11. 手工验证：
   - 创建/切换会话
   - 提交消息
   - 聚焦子执行
   - 打开子会话
   - 关闭子会话

## Rollback Considerations

- 不建议回滚到“双轨状态模型 + descriptor downgrade + legacy cancel route”的状态，因为那会重新引入支持面歧义。
- 如果某条被删除的 surface 真有遗漏消费者，应先补 caller inventory 与 owner 说明，再重新设计，而不是直接恢复旧壳。
