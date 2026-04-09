# Contract: Agent Collaboration Tools

## 目的

定义模型可调用的 child-session 协作工具契约，确保：

- prompt surface 单一
- 权限/审计语义一致
- runtime 内部 transport 细节不泄漏到模型层

## Tool Set

### 1. `spawnAgent`

**Purpose**  
创建新的 child agent / child session。

**Input**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `type` | `string` | No | 目标 profile，默认 `explore` 或 runtime 配置默认值 |
| `description` | `string` | Yes | 仅供 UI/日志展示 |
| `prompt` | `string` | Yes | child 收到的任务主体 |
| `context` | `string` | No | 补充上下文 |

**Output**

- `success=true`
- `agentRef`
- `status=running`
- `summary`
- `openSessionId`

### 2. `sendAgent`

**Purpose**  
向既有 child agent 追加要求或返工请求。

**Input**

| Field | Type | Required |
|-------|------|----------|
| `agentId` | `string` | Yes |
| `message` | `string` | Yes |
| `context` | `string` | No |

**Output**

- `success=true`
- `accepted=true`
- `agentRef`
- `deliveryId`

### 3. `waitAgent`

**Purpose**  
等待指定 child agent 到达下一个可消费状态。

**Input**

| Field | Type | Required |
|-------|------|----------|
| `agentId` | `string` | Yes |
| `until` | `final \| next_delivery` | No |

**Output**

- `status=running|completed|failed|aborted|cancelled`
- `agentRef`
- `summary`
- `finalReply` 或 `failure`

### 4. `closeAgent`

**Purpose**  
关闭指定 child agent 或其子树。

**Input**

| Field | Type | Required |
|-------|------|----------|
| `agentId` | `string` | Yes |
| `cascade` | `boolean` | No, default `true` |

**Output**

- `accepted=true`
- `closedRootAgentId`
- `cascade=true|false`

### 5. `resumeAgent`

**Purpose**  
恢复一个已完成但仍可继续协作的 child agent。

**Input**

| Field | Type | Required |
|-------|------|----------|
| `agentId` | `string` | Yes |
| `message` | `string` | No |

**Output**

- `accepted=true`
- `agentRef`
- `status=running`

### 6. `deliverToParent`

**Purpose**  
仅 child session 可见，用于把阶段性结果或最终交付送回直接父 agent。

**Input**

| Field | Type | Required |
|-------|------|----------|
| `summary` | `string` | Yes |
| `findings` | `string[]` | No |
| `finalReply` | `string` | No |
| `artifacts` | `object[]` | No |

**Output**

- `accepted=true`
- `parentAgentId`
- `deliveryId`

## 约束

1. 所有工具输入都必须以稳定 `agentId` / `agentRef` 为目标，不允许依赖 UI path。
2. 所有工具输出都必须是可消费结果语义，不能返回 runtime 原始 envelope 或 raw JSON。
3. `deliverToParent` 只能投递给直接父 agent。
4. `waitAgent` 不能阻塞无关 child。
5. 相同协作请求必须具备幂等与去重语义。

## Runtime Mapping

这些工具在 runtime 内部必须映射到：

- durable `AgentInboxEnvelope`
- target agent 唤醒 / 排队
- 必要时的 `CollaborationNotification`

但这种内部映射不应在 tool 结果里直接暴露。
