# 多会话前端架构设计

## 需求

前端需要**自由切换**主会话和子 Agent 会话，包括：
1. 查看子 Agent 的完整消息历史
2. 在子 Agent 会话中流式接收消息
3. 在主会话和子会话之间导航
4. 支持嵌套多层子 Agent

## 核心概念

### 会话树结构

```
Root Session (root-agent-xxx)
├── Turn 1: 用户问"分析代码"
├── Turn 2: 调用 runAgent
│   └── SubRun (explore-agent-yyy)  ← 可切换到此会话
│       ├── Turn 1: "搜索文件"
│       ├── Turn 2: "读取文件"
│       └── Turn 3: 返回结果
└── Turn 3: 主 Agent 总结
```

### 两个关键设计点

1. **每个会话独立存储**（已有 `IndependentSession` 模式）
2. **子会话事件独立推送**（子会话有自己的 SSE 流）

## 设计方案

### 方案概览

```
┌─────────────────────────────────────────────────────────────┐
│                        Frontend                              │
├─────────────────────────────────────────────────────────────┤
│  ┌───────────────┐    ┌─────────────────────────────────┐  │
│  │ SessionTree   │    │    ActiveSessionManager          │  │
│  │ - 父子关系     │───▶│ - 当前查看的会话                  │  │
│  │ - 会话元数据   │    │ - 会话切换历史（支持前进/后退）     │  │
│  └───────────────┘    │ - 每个 session 一个 SSE 连接       │  │
│                       └─────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
         │                                          │
         │ HTTP API                                 │
         │ /sessions                                │ SSE /sessions/{id}/events
         ▼                                          ▼
┌─────────────────────────────────────────────────────────────┐
│                     Backend Runtime                           │
├─────────────────────────────────────────────────────────────┤
│  SessionManager                                               │
│  - get_session(id)     → SessionState                         │
│  - list_sessions()      → Vec<SessionMeta> (含 parent info)   │
│  - get_session_tree()   → SessionTree                          │
└─────────────────────────────────────────────────────────────┘
```

### 1. 后端：会话树查询 API

#### 1.1 扩展 `SessionMeta` 添加子会话信息

```rust
// crates/core/src/session/mod.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub working_dir: String,
    pub display_name: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    
    // 已有字段
    pub parent_session_id: Option<String>,
    pub parent_storage_seq: Option<u64>,
    
    // 新增：子会话信息
    #[serde(default)]
    pub child_sessions: Vec<ChildSessionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSessionMeta {
    pub session_id: String,
    pub sub_run_id: String,      // 关联到 SubRunHandle
    pub agent_profile: String,   // 哪个 agent 创建的
    pub created_at: String,
    pub storage_mode: SubRunStorageMode,
}
```

#### 1.2 新增会话树查询 API

```rust
// GET /api/v1/sessions/tree
//
// Response:
{
    "root": {
        "sessionId": "root-123",
        "title": "主会话",
        "createdAt": "2026-04-06T10:00:00Z",
        "childSessions": [
            {
                "sessionId": "child-456",
                "subRunId": "subrun-abc",
                "agentProfile": "explore",
                "createdAt": "2026-04-06T10:01:00Z",
                "storageMode": "independentSession",
                "childSessions": []  // 支持多层嵌套
            }
        ]
    }
}
```

#### 1.3 子会话事件独立推送

**关键**：每个子会话（无论是 `SharedSession` 还是 `IndependentSession`）都有自己的 SSE 端点：

```
GET /api/v1/sessions/{session_id}/events
```

**修改点**：
- `SharedSession` 模式下，子 Agent 的事件写入**同一个 session log**
- 但 SSE 推送时，根据 `AgentEventContext.sub_run_id` **区分事件归属**
- 前端可以**过滤只看特定 sub_run_id 的事件**

### 2. 前端：多会话管理

#### 2.1 会话树状态管理

```typescript
// frontend/src/types.ts

export interface SessionNode {
  session: Session;
  children: SessionNode[];
  parent: SessionNode | null;
}

export interface SessionTree {
  roots: SessionNode[];  // 可能同时有多个根会话
  lookup: Map<string, SessionNode>;  // 快速查找
}

// Reducer
export interface SessionsState {
  tree: SessionTree;
  
  // 当前激活的会话栈（支持导航）
  // 例如：[root-123, child-456]
  // 表示当前正在查看 child-456，可以从这里回到 root-123
  activeStack: string[];
  
  // 每个 session 的 SSE 连接状态
  connections: Map<string, EventSource>;
}
```

#### 2.2 会话切换 UI

```typescript
// frontend/src/components/Session/SessionNav.tsx

export function SessionNav() {
  const { tree, activeStack } = useSessionsState();
  const currentSessionId = activeStack[activeStack.length - 1];
  
  // 构建面包屑导航
  const breadcrumbs = useMemo(() => {
    const nodes: SessionNode[] = [];
    let current = tree.lookup.get(currentSessionId);
    while (current) {
      nodes.unshift(current);
      current = current.parent;
    }
    return nodes;
  }, [tree, currentSessionId]);
  
  return (
    <nav className="session-nav">
      {breadcrumbs.map((node, index) => (
        <Fragment key={node.session.id}>
          <button
            onClick={() => switchToSession(node.session.id)}
            className={index === breadcrumbs.length - 1 ? 'active' : ''}
          >
            {node.session.title}
          </button>
          {index < breadcrumbs.length - 1 && <span＞ › </span>}
        </Fragment>
      ))}
    </nav>
  );
}
```

#### 2.3 子会话卡片（在主会话中显示）

```typescript
// frontend/src/components/Chat/SubRunCard.tsx

export function SubRunCard({ subRunStart, subRunFinish }: Props) {
  const childSessionId = subRunStart.childSessionId;
  const isIndependent = subRunStart.storageMode === 'independentSession';
  
  return (
    <div className="subrun-card">
      <div className="subrun-header">
        <Icon name="agent" />
        <span>Agent: {subRunStart.agentProfile}</span>
        <span className="subrun-id">{subRunStart.subRunId}</span>
      </div>
      
      {/* 可展开查看子会话 */}
      <button onClick={() => openChildSession(subRunStart)}>
        查看完整会话 →
      </button>
      
      {/* 如果是独立会话，显示摘要 */}
      {isIndependent && subRunFinish && (
        <div className="subrun-summary">
          <StatusBadge status={subRunFinish.result.status} />
          <p>{subRunFinish.result.summary}</p>
        </div>
      )}
    </div>
  );
}
```

#### 2.4 SSE 连接管理

```typescript
// frontend/src/lib/sessionEvents.ts

export class SessionEventManager {
  private connections = new Map<string, SessionConnection>();
  
  // 为指定会话建立 SSE 连接
  connect(sessionId: string, onEvent: (event: AgentEvent) => void) {
    // 如果已连接，复用
    if (this.connections.has(sessionId)) {
      return;
    }
    
    const eventSource = new EventSource(`/api/v1/sessions/${sessionId}/events`);
    
    eventSource.onmessage = (e) => {
      const event = parseAgentEvent(e.data);
      
      // 根据会话模式处理事件
      if (isSharedSessionMode(event)) {
        // SharedSession: 过滤只属于当前 session 或 sub_run 的事件
        if (shouldProcessEvent(event, sessionId)) {
          onEvent(event);
        }
      } else {
        // IndependentSession: 直接处理
        onEvent(event);
      }
    };
    
    this.connections.set(sessionId, {
      eventSource,
      sessionId,
      listeners: new Set([onEvent]),
    });
  }
  
  disconnect(sessionId: string) {
    const conn = this.connections.get(sessionId);
    if (conn) {
      conn.eventSource.close();
      this.connections.delete(sessionId);
    }
  }
}

// SharedSession 模式下的事件过滤
function shouldProcessEvent(event: AgentEvent, currentSessionId: string): boolean {
  const agentContext = event.data.agent;
  
  // 没有 agent 上下文 → 根会话事件
  if (!agentContext) {
    return true;
  }
  
  // 有 sub_run_id → 检查是否属于当前查看的 sub_run
  if (agentContext.subRunId) {
    // 这需要配合前端维护"当前查看的 sub_run_id"状态
    return belongsToCurrentView(agentContext, currentSessionId);
  }
  
  // 有 agent_id 但没有 sub_run_id → 根会话的 agent 事件
  return true;
}
```

### 3. 后端实现：SharedSession 事件过滤

#### 3.1 SSE 端点支持 sub_run 过滤

```rust
// GET /api/v1/sessions/{session_id}/events?sub_run={sub_run_id}
//
// 如果指定 sub_run_id，只推送该 sub_run 的事件
// 否则推送整个 session 的事件

pub async fn session_events_sse(
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<ApiState>>,
) -> Result<Sse<SessionEventStream>, ApiError> {
    let sub_run_filter = params.get("sub_run").map(|s| s.to_string());
    
    let stream = SessionEventStream::new(
        session_id,
        state.event_tx.clone(),
        sub_run_filter,
    );
    
    Ok(Sse::new(stream))
}

struct SessionEventStream {
    session_id: String,
    event_tx: BroadcastSender<SessionEventRecord>,
    sub_run_filter: Option<String>,
    rx: Receiver<SessionEventRecord>,
}

impl SessionEventStream {
    fn new(
        session_id: String,
        event_tx: BroadcastSender<SessionEventRecord>,
        sub_run_filter: Option<String>,
    ) -> Self {
        let rx = event_tx.subscribe();
        Self {
            session_id,
            event_tx,
            sub_run_filter,
            rx,
        }
    }
}

impl Stream for SessionEventStream {
    type Item = Result<Event, Infallible>;
    
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            match self.rx.poll_next_unpin(cx) {
                Poll::Ready(Some(record)) => {
                    // 过滤：只推送匹配 session_id 和 sub_run_id 的事件
                    if record.session_id != self.session_id {
                        continue;
                    }
                    
                    if let Some(filter_sub_run) = &self.sub_run_filter {
                        if !event_belongs_to_sub_run(&record.event, filter_sub_run) {
                            continue;
                        }
                    }
                    
                    let event = to_sse_event(record);
                    return Poll::Ready(Some(Ok(event)));
                },
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

fn event_belongs_to_sub_run(event: &AgentEvent, sub_run_id: &str) -> bool {
    match event {
        AgentEvent::UserMessage { agent, .. }
        | AgentEvent::Assistant { agent, .. }
        | AgentEvent::ToolCall { agent, .. } => {
            match &agent.sub_run_id {
                Some(id) => id == sub_run_id,
                None => false,
            }
        },
        AgentEvent::SubRunStarted { agent, .. }
        | AgentEvent::SubRunFinished { agent, .. } => {
            match &agent.sub_run_id {
                Some(id) => id == sub_run_id,
                None => false,
            }
        },
        _ => false,
    }
}
```

### 4. 前端导航流程

```typescript
// 场景：用户点击"查看子会话"

// 1. 用户点击子会话卡片
function openChildSession(subRunStart: SubRunStartMessage) {
  const childSessionId = subRunStart.childSessionId;
  
  if (!childSessionId) {
    // SharedSession 模式：使用 sub_run 过滤
    switchToSubRunView(subRunStart.subRunId!);
  } else {
    // IndependentSession 模式：切换到独立会话
    switchToSession(childSessionId);
  }
}

// 2. 切换到 SharedSession 的 sub_run 视图
function switchToSubRunView(subRunId: string) {
  // 更新激活栈
  dispatch({
    type: 'PUSH_ACTIVE_VIEW',
    view: {
      type: 'sub-run',
      sessionId: currentRootSessionId,
      subRunId,
    }
  });
  
  // 建立/复用 SSE 连接，带上 sub_run 过滤
  eventManager.connect(currentRootSessionId, {
    subRunId,
    onEvent: handleEvent,
  });
}

// 3. 切换到独立子会话
function switchToSession(sessionId: string) {
  // 加载会话历史
  const session = await fetchSession(sessionId);
  
  // 更新激活栈
  dispatch({
    type: 'PUSH_ACTIVE_SESSION',
    sessionId,
  });
  
  // 建立 SSE 连接
  eventManager.connect(sessionId, {
    onEvent: handleEvent,
  });
}
```

### 5. UI 展示

#### 5.1 主会话视图

```
┌─────────────────────────────────────────────────┐
│ 会话导航: Root 会话 › Explore Agent             │
├─────────────────────────────────────────────────┤
│                                                 │
│ 🧒 用户: 分析这个代码库                         │
│                                                 │
│ 🤖 Agent: 我来分析...                           │
│                                                 │
│ ┌───────────────────────────────────────────┐  │
│ │ 🔧 Explore Agent                          │  │
│ │ 正在搜索文件...                           │  │
│ │                                          │  │
│ │ [查看完整会话 →]                          │  │
│ └───────────────────────────────────────────┘  │
│                                                 │
│ 🤖 Agent: 根据探索结果，代码库包含...         │
│                                                 │
└─────────────────────────────────────────────────┘
```

#### 5.2 子会话视图

```
┌─────────────────────────────────────────────────┐
│ 会话导航: Root 会话 › Explore Agent             │
│ [← 返回父会话]                                   │
├─────────────────────────────────────────────────┤
│                                                 │
│ 🧒 父 Agent: 分析这个代码库                     │
│                                                 │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  │
│                                                 │
│ 🧒 任务: 搜索所有 TypeScript 文件              │
│                                                 │
│ 🔧 Tool: glob("**/*.ts")                       │
│ 📦 Found 42 files                              │
│                                                 │
│ 🤖 找到了 42 个 TypeScript 文件...            │
│                                                 │
│ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  │
│                                                 │
└─────────────────────────────────────────────────┘
```

## 实现步骤

### 阶段 1：后端 API（1-2 天）
1. 扩展 `SessionMeta` 添加 `child_sessions` 字段
2. 实现 `GET /api/v1/sessions/tree` API
3. SSE 端点支持 `sub_run` 查询参数过滤

### 阶段 2：前端状态管理（1-2 天）
1. 设计 `SessionsState` 数据结构
2. 实现 `sessionTree` reducer
3. 实现会话切换逻辑

### 阶段 3：前端 UI（2-3 天）
1. `SessionNav` 面包屑导航组件
2. `SubRunCard` 子会话卡片组件
3. 子会话独立视图
4. SSE 连接管理优化

### 阶段 4：测试与优化（1-2 天）
1. 多层嵌套测试
2. SharedSession vs IndependentSession 模式测试
3. 性能优化（避免重复连接）

## 优势

1. **清晰的会话层级**：用户能直观看到父子关系
2. **灵活的导航**：自由切换任意会话
3. **流式体验**：子会话消息实时流式显示
4. **向后兼容**：不影响现有单会话逻辑
5. **可扩展**：支持多层嵌套子 Agent

## 关键技术点

1. **SSE 连接复用**：同一 session 只建立一次连接，通过过滤参数控制
2. **事件归属判断**：根据 `AgentEventContext.sub_run_id` 判断事件归属
3. **状态同步**：确保会话树状态与后端一致
4. **性能优化**：只加载可见会话的消息历史
