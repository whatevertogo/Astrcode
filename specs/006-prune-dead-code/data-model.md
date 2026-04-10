# Data Model: 删除死代码与兼容层收口

本特性不是新增业务领域，而是为“支持面审计、迁移与删除”建立一套清晰模型，避免实现阶段继续靠口头判断。

## 1. `SupportSurface`

表示一个当前对外或对内有独立语义的 surface，可以是：

- 前端导出 API
- 前端 projection / helper
- server HTTP route
- runtime / protocol 公开语义
- live 文档中被宣称存在的能力

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `surface_id` | `String` | Yes | 唯一标识 |
| `layer` | `frontend \| server \| runtime \| protocol \| docs \| tests` | Yes | 所属层 |
| `kind` | `api \| projection \| route \| type \| contract \| fixture` | Yes | surface 类型 |
| `name` | `String` | Yes | 人类可读名称 |
| `owner_boundary` | `String` | No | 当前负责边界；未知则为空 |
| `status` | `inventoried \| retained \| migrate_then_remove \| remove_now \| removed` | Yes | 当前清理状态 |
| `reason` | `String` | Yes | 保留或删除原因 |

## 2. `SurfaceConsumer`

表示某个 surface 的真实消费者。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `consumer_id` | `String` | Yes | 唯一标识 |
| `consumer_kind` | `product_flow \| ui_component \| internal_service \| external_contract \| test_only \| doc_only` | Yes | 消费者类型 |
| `name` | `String` | Yes | 名称 |
| `active` | `bool` | Yes | 是否属于当前主线 |

**Validation**

- 只有 `active=true` 的 `product_flow`、`ui_component`、`internal_service` 或明确 owner 的 `external_contract` 才能成为保留理由。
- `test_only` 和 `doc_only` 不能单独构成保留理由。

## 3. `SurfaceDecision`

表示对某个 surface 的最终判定。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `surface_id` | `String` | Yes | 对应 `SupportSurface` |
| `decision` | `retain \| migrate_then_remove \| remove` | Yes | 决策结果 |
| `replacement_surface_id` | `Option<String>` | No | 若需迁移，指向替代入口 |
| `blocking_consumer_id` | `Option<String>` | No | 若不能立即删除，指出阻塞删除的当前消费者 |
| `verification_rule` | `String` | Yes | 如何证明该决策已落地 |

## 4. `SummaryFact`

表示本次清理中涉及的摘要语义单元。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `summary_id` | `String` | Yes | 唯一标识 |
| `role` | `handoff_fact \| notification_fact \| projection_only` | Yes | 摘要角色 |
| `consumed_by_ui` | `bool` | Yes | 是否被当前 UI 消费 |
| `consumed_by_runtime` | `bool` | Yes | 是否被当前运行时消费 |
| `decision` | `retain \| remove` | Yes | 清理决策 |

**Validation**

- `projection_only` 且无当前消费者的摘要必须删除。
- `handoff_fact` 与 `notification_fact` 若仍被消费，必须保留。

## 5. `LegacyArtifact`

表示仍暴露在主线中的 legacy 工件。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `artifact_id` | `String` | Yes | 唯一标识 |
| `domain` | `history \| subrun \| control \| docs \| tests` | Yes | 所属领域 |
| `artifact_kind` | `route \| type \| downgrade_state \| fixture \| doc_statement` | Yes | 工件类型 |
| `current_behavior` | `active_flow \| unsupported_but_public \| orphaned` | Yes | 当前行为 |
| `failure_strategy` | `migrate_then_delete \| explicit_failure \| direct_delete` | Yes | 清理策略 |

## 6. `ValidationScenario`

表示本次清理后的验收场景。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `scenario_id` | `String` | Yes | 唯一标识 |
| `category` | `retained_flow \| deletion_assertion \| doc_sync` | Yes | 场景类型 |
| `description` | `String` | Yes | 验证描述 |
| `must_pass` | `bool` | Yes | 是否为 release gate |

## Relationships

- 一个 `SupportSurface` 可以有零个或多个 `SurfaceConsumer`。
- 一个 `SupportSurface` 必须对应一个 `SurfaceDecision`。
- 某些 `SupportSurface` 同时也是 `LegacyArtifact`。
- 某些 `SupportSurface` 关联一个 `SummaryFact`。
- 每个 `SurfaceDecision` 必须至少被一个 `ValidationScenario` 覆盖。

## State Transitions

### `SupportSurface.status`

`inventoried -> retained | migrate_then_remove | remove_now -> removed`

- `retained` 的 surface 不进入删除流，但必须在 live docs 中保留正确说明。
- `migrate_then_remove` 必须先完成替代入口切换，再进入 `removed`。
- `remove_now` 可直接进入 `removed`，前提是没有 active consumer。

### `LegacyArtifact.failure_strategy`

`migrate_then_delete` 适用于当前主线仍依赖的 legacy 入口。  
`explicit_failure` 适用于已不支持、但仍需清晰失败的旧输入。  
`direct_delete` 适用于没有当前消费者的 orphan artifact。
