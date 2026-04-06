# Agent Loop 内容架构

## 概要

本文档定义了 Astrcode Agent Loop 中消息、工具使用、思考等内容的数据模型和组织结构。

---

## 核心数据模型

### Session（会话）

```rust
/// 会话表示一次独立的对话实例
pub struct Session {
    /// 会话唯一标识
    pub session_id: Uuid,
    /// 会话标题（可选，AI 生成或用户手动设置）
    pub title: Option<String>,
    /// 所属项目路径
    pub project_path: PathBuf,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 最后活跃时间
    pub last_activity_at: DateTime<Utc>,
    /// 会话元数据（可选）
    pub metadata: Option<SessionMetadata>,
}

/// 会话元数据
pub struct SessionMetadata {
    /// 父会话 ID（子会话时设置）
    pub parent_session_id: Option<Uuid>,
    /// 父会话中的 turn_id（子会话时设置）
    pub parent_turn_id: Option<String>,
    /// 子会话摘要（父会话视角）
    pub child_session_summary: Option<String>,
}
```

### SubSession（子会话）

```rust
/// 子会话表示由主会话通过 spawnAgent 工具创建的独立会话实例
pub struct SubSession {
    /// 子会话自己的 session_id
    pub session_id: Uuid,
    /// 创建者（父会话的 session_id）
    pub parent_session_id: Uuid,
    /// 父会话中触发创建的 turn_id
    pub parent_turn_id: String,
    /// 子会话标题（可选，AI 生成或基于任务自动生成）
    pub title: Option<String>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 最后活跃时间
    pub last_activity_at: DateTime<Utc>,
}
```

### Message（消息）

```rust
/// 会话中的一条消息
pub struct Message {
    /// 消息唯一标识
    pub id: String,
    /// 所属会话 ID
    pub session_id: Uuid,
    /// 消息角色
    pub role: MessageRole,
    /// 消息内容块列表
    pub blocks: Vec<ContentBlock>,
    /// 消息元数据
    pub metadata: Option<MessageMetadata>,
    /// 父消息 ID（可选）
    pub parent_id: Option<String>,
}

/// 消息角色
pub enum MessageRole {
    User,
    Assistant,
}

/// 内容块：消息由多个内容块组成
pub enum ContentBlock {
    Text(TextBlock),
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
    Thinking(ThinkingBlock),
}

/// 纯文本内容
pub struct TextBlock {
    pub text: String,
}

/// 工具调用
pub struct ToolUseBlock {
    /// 工具唯一调用 ID
    pub tool_use_id: String,
    /// 工具名称
    pub name: String,
    /// 工具调用参数（JSON 原始值）
    pub input: JsonValue,
}

/// 工具调用结果
pub struct ToolResultBlock {
    /// 对应的 tool_use_id
    pub tool_use_id: String,
    /// 工具返回内容
    pub content: Vec<ToolResultContent>,
    /// 工具执行状态
    pub status: ToolResultStatus,
}

pub enum ToolResultContent {
    Text(String),
    Image { data: String, format: String, detail: Option<ImageDetail> },
}

pub enum ToolResultStatus {
    Success,
    Error,
    Interrupted,
}

/// 思考内容
pub struct ThinkingBlock {
    pub thinking: String,
}
```

### MessageMetadata（消息元数据）

```rust
pub struct MessageMetadata {
    /// 消息创建时间
    pub created_at: DateTime<Utc>,
    /// 模型名称（仅 Assistant）
    pub model: Option<String>,
    /// 停止原因（仅 Assistant）
    pub stop_reason: Option<String>,
    /// Token 使用情况
    pub token_usage: Option<TokenUsage>,
    /// 子会话关联（当工具调用触发子会话时）
    pub child_session_id: Option<Uuid>,
    /// 代理实例 ID（多代理时区分）
    pub agent_instance_id: Option<Uuid>,
}
```

---

## 消息持久化

### SessionRepository trait

```rust
/// 会话持久化接口
pub trait SessionRepository: Send + Sync {
    /// 创建新会话
    async fn create_session(&self, session: Session) -> Result<Session, StorageError>;
    
    /// 获取会话
    async fn get_session(&self, session_id: &str) -> Result<Session, StorageError>;
    
    /// 更新会话（标题、活跃时间等）
    async fn update_session(&self, session_id: &str, updates: SessionUpdate) -> Result<Session, StorageError>;
    
    /// 获取会话的消息列表
    async fn list_messages(&self, session_id: &str, limit: usize, before_id: Option<&str>) -> Result<Vec<Message>, StorageError>;
    
    /// 追加消息
    async fn add_message(&self, message: Message) -> Result<Message, StorageError>;
    
    /// 更新消息
    async fn update_message(&self, message: Message) -> Result<Message, StorageError>;
    
    /// 获取会话的所有工具调用
    async fn list_tool_calls(&self, session_id: &str) -> Result<Vec<ToolHistory>, StorageError>;
}

/// 字段更新（只发变化部分）
pub struct SessionUpdate {
    pub title: Option<String>,
    pub metadata: Option<SessionMetadata>,
}

/// 工具调用历史
pub struct ToolHistory {
    /// 工具调用 ID
    pub tool_call_id: String,
    /// 工具名称
    pub name: String,
    /// 调用参数（JSON 原始值）
    pub args: JsonValue,
    /// 调用结果
    pub result: String,
    /// 调用状态
    pub status: ToolResultStatus,
    /// 所属消息 ID
    pub message_id: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}
```

### 文件存储

```rust
/// 文件存储实现：每会话一个文件夹
///
/// 目录结构：
/// sessions/
///   └─ {session_id}/
///       ├── session.json         # 会话基础信息
///       ├── sub_sessions.json    # 子会话列表（如果是父会话）
///       └─ messages.jsonl        # 消息行式存储
pub struct FileSessionRepository {
    /// 存储基础路径
    base_path: PathBuf,
    // ...
}
```

**子会话的独立存储：**
```
sessions/
  └─ {parent_session_id}/
      ├── session.json           # 父会话基础信息
      ├── sub_sessions.json      # 子会话元数据列表
      └── messages.jsonl         # 父会话消息

  └─ {child_session_id}/
      ├── session.json           # 子会话基础信息（含 parent_session_id）
      └── messages.jsonl         # 子会话的完整消息记录
```

### 数据库存储

```rust
/// 数据库存储实现：关系型/文档型
pub struct DbSessionRepository {
    db: Arc<dyn Database>,
}
```

#### 会话主表 (sessions)

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | UUID | 会话唯一标识 |
| `title` | VARCHAR | 会话标题 |
| `project_path` | TEXT | 所属项目路径 |
| `is_sub_session` | BOOLEAN | 是否为子会话 |
| `parent_session_id` | UUID NULL | 父会话 ID（子会话时） |
| `parent_turn_id` | VARCHAR NULL | 父会话的 turn_id（子会话时） |
| `summary` | TEXT NULL | 子会话摘要（父会话可见） |
| `created_at` | TIMESTAMPTZ | 创建时间 |
| `updated_at` | TIMESTAMPTZ | 更新时间 |

#### 消息表 (messages)

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | VARCHAR | 消息唯一标识 |
| `session_id` | UUID | 所属会话 ID |
| `turn_id` | VARCHAR | 对话轮次 ID |
| `role` | VARCHAR | 消息角色：user/assistant |
| `blocks` | JSONB | 内容块列表 |
| `metadata` | JSONB | 消息元数据 |
| `parent_id` | VARCHAR NULL | 父消息 ID |
| `sequence` | INT | 消息排序（用于有序加载） |
| `created_at` | TIMESTAMPTZ | 创建时间 |

**索引：**
- `(session_id, turn_id, sequence)` 复合索引：按轮次有序加载消息
- `(session_id, turn_id, message_index)` 覆盖索引：用于快速跳转

#### 事件日志表 (event_logs, 保留用于审计/回放)

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | BIGSERIAL | 事件 ID |
| `session_id` | UUID | 所属会话 ID |
| `agent_id` | UUID NULL | 代理实例 ID |
| `timestamp` | TIMESTAMPTZ | 事件时间 |
| `kind` | VARCHAR | 事件类型 |
| `data` | JSONB | 事件详细数据 |
| `parent_turn_id` | VARCHAR NULL | 父 turn 编号（子 agent） |
| `agent_profile` | VARCHAR NULL | Agent Profile 名称 |

---

## LLM 交互与内容转换

### LLM API 到 ContentBlock

```rust
/// 将 LLM 返回的消息内容转换为 ContentBlock 列表
pub fn llm_message_to_content_blocks(
    llm_message: LlmMessage,
    // ...
) -> Vec<ContentBlock> {
    // ...
}

/// 将 ContentBlock 列表转换为 LLM 请求的内容格式
pub fn content_blocks_to_llm_format(
    blocks: &[ContentBlock],
) -> LlmMessageContent {
    // ...
}
```

### Token 统计

```rust
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}
```

---

## 子会话（SubSession）

### 子会话的创建

子会话在以下情况下创建：

1. **Agent 调用 `spawnAgent` 工具**
2. **LLM 返回包含子 agent 调用的工具使用块**

子会话创建后会：
1. 创建新的 `SessionRecord` 标记为 is_sub_session = true
2. 创建对应的 `SubSessionRecord` 关联父会话
3. 返回子会话 ID 给调用方

### 子会话的消息同步

子会话在运行时会：
1. 在子会话的 `messages.jsonl` 中追加消息
2. 子会话运行完成后，将摘要写入父会话消息的 `child_session_id`
3. 父会话可选择展示子会话摘要或展开子会话消息

### 子会话的折叠与展开

子会话的折叠与展开由 **前端** 决定是否折叠展示：

- **默认折叠**：显示 `[SubSession: Explore code]` + 摘要 + 耗时/步数
- **展开后**：显示子会话完整消息流

---

## 会话查询 API

### 获取会话的消息

```
GET /api/sessions/{session_id}/messages
    ?limit=50           # 每次加载的消息数
    &before_id=xxx      # 分页游标
```

**响应：**
```json
{
  "messages": [...],
  "has_more": true
}
```

### 获取会话的子会话列表

```
GET /api/sessions/{session_id}/sub_sessions
```

**响应：**
```json
{
  "sub_sessions": [
    {
      "id": "...",
      "session_id": "...",
      "parent_session_id": "...",
      "parent_turn_id": "turn-5",
      "title": "探索代码库",
      "created_at": "2025-12-31T00:00:00Z",
      "summary": "共执行 5 步，发现 3 处使用点",
      "status": "completed"
    }
  ]
}
```

### 获取子会话的消息

```
GET /api/sessions/{sub_session_id}/messages
    ?limit=50
    &before_id=xxx
```

返回子会话的完整消息列表。

---

## 前端展示

### 消息渲染

会话中的每条消息都渲染为对应的内容块：

```
╔══════════════════════════════════════╗
║  👤 用户消息 (20:53)                ║
║  我想重构这个模块                    ║
╠══════════════════════════════════════╣
║  🤖 助手消息 (20:53) [model]        ║
║  好的，让我先分析一下当前的代码...   ║
║                                     ║
║  🔧 [thinking]                     ║
║  需要先了解模块结构和依赖关系...     ║
║                                     ║
║  🔧 Tool: spawnAgent                  ║
║  ┌─────────────────────────────┐    ║
║  │ 🧩 [SubSession: Explore] ↓  │    ║
║  │ 共 5 步 | 耗时 12s           │    ║
║  │ 发现 3 处相关使用点...       │    ║
║  └─────────────────────────────┘    ║
║                                     ║
║  根据 explore 结果，我看到...        ║
╚══════════════════════════════════════╝
```

### 工具调用特殊处理

`spawnAgent` 工具调用渲染为 **SubSession 卡片**：

**折叠状态：**
```
┌─────────────────────────────────────┐
│ 🔍 [SubSession: Explore]           │
│ 共 5 步 | 耗时 12s                  │
│ 摘要：发现 3 处相关使用点...        │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━    │
│ ▶ 点击展开详情                       │
└─────────────────────────────────────┘
```

**展开状态：**
```
┌─────────────────────────────────────┐
│ 🔍 [SubSession: Explore]            ▼│
│ 共 5 步 | 耗时 12s                  │
│ ─────────────────────────────────   │
│   ╔═══════════════════════════╗     │
│   │  🤖 Agent 消息            │     │
│   │  让我分析一下代码...      │     │
│   │                           │     │
│   │  🔧 Tool: read_file       │     │
│   │  src/models/user.rs       │     │
│   │                           │     │
│   │  🔧 Tool: grep            │     │
│   │  pattern: "UserTrait"     │     │
│   │                           │     │
│   │  🔧 Tool: semantic_search │     │
│   │  query: "user module"     │     │
│   ════════════════════════════╝     │
│                                     │
│  根据 explore 结果，我看到...        │
└─────────────────────────────────────┘
```

### 折叠状态的持久化

折叠状态由前端本地存储管理（如 `localStorage`），不保存到 Session。

---

## 消息加载策略

### 分页加载

为优化性能，消息采用分页加载：

- **初始加载**：最近 50 条消息
- **向下滚动**：加载更早的消息
- **向上滚动**：（可选）预加载更新的消息或新轮次

### 按需加载内容

对于 `ToolResultBlock` 中的图片等内容，可按需加载：

```rust
// 先加载缩略图/占位符
message.blocks.iter()
    .filter_map(|block| match block {
        ContentBlock::ToolResult(r) if r.has_image() => {
            Some(r.get_thumbnail_url())
        }
        _ => None,
    })

// 用户点击后加载完整图片
message.get_tool_result_image(tool_use_id, high_resolution: true)
```

---

## 多 Agent 支持

### 会话中的 Agent 实例

每个 Agent 实例有独立的 `session_id`：
- 用户直接对话的 Agent 使用根 session
- 子 Agent 创建新的会话，并关联到父会话

### 消息路由

```
Event { session_id, agent_instance_id, ... }
    → 写入对应的 Session 的消息列表
```

### 上下文传递

当 Agent A 调用 Agent B 时：
1. Agent A 创建一个子会话（SubSession）
2. 子会话继承工作目录等上下文
3. Agent B 在子会话中独立运行
4. 子会话完成后，结果以摘要形式同步到 Agent A 的消息
5. 前端可选择展示完整子会话或仅显示摘要
