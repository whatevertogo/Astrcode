# 多会话前端架构 - 完整实施计划

## 目标

实现前端可以自由切换主会话和子 Agent 会话，支持流式显示子 Agent 的执行过程。

## 当前状态分析

### ✅ 已有基础设施

1. **SSE 事件流**：`GET /api/sessions/{id}/events`
   - 支持断点续传（`after_event_id`）
   - 返回完整会话事件流

2. **前端消息类型**：
   - `SubRunStartMessage` / `SubRunFinishMessage`
   - 包含 `childSessionId` / `subRunId` / `agentProfile`

3. **会话存储模式**：
   - `SharedSession`：子 Agent 事件写入主会话 log
   - `IndependentSession`：子 Agent 有独立会话

### ❌ 缺失部分

1. **子会话查询 API**：无法获取某会话的所有子会话列表
2. **SSE 事件过滤**：`SharedSession` 模式下无法过滤只看子会话事件
3. **前端会话树管理**：缺少导航栈和多会话状态管理
4. **前端子会话 UI**：缺少子会话卡片和导航组件

---

## 架构设计

### 数据流架构

```
┌─────────────────────────────────────────────────────────────┐
│                        Frontend                              │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           SessionStateManager                       │  │
│  │  - tree: SessionTree                                │  │
│  │  - activeStack: string[] (导航历史)                  │  │
│  │  - connections: Map<sessionId, SSEConnection>        │  │
│  └──────────────────────────────────────────────────────┘  │
│                          │                                  │
│                          ▼                                  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           SSEConnectionManager                       │  │
│  │  - 为每个活跃 session 维护 SSE 连接                   │  │
│  │  - 支持事件过滤（sub_run_id）                         │  │
│  └──────────────────────────────────────────────────────┘  │
│                          │                                  │
└──────────────────────────┼──────────────────────────────────┘
                           │
            ┌──────────────┼──────────────┐
            │              │              │
            ▼              ▼              ▼
    ┌───────────┐  ┌───────────┐  ┌───────────┐
    │ GET /api  │  │ GET /api  │  │ GET /api  │
    │ /sessions │  │ /sessions │  │ /sessions │
    │ /tree     │  │ /{id}     │  │ /{id}/... │
    └───────────┘  │ /children │  │ /events   │
                   └───────────┘  └───────────┘
            │              │              │
            ▼              ▼              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Backend Runtime                           │
├─────────────────────────────────────────────────────────────┤
│  SessionManager                                               │
│  - get_session_tree()  → SessionTree                         │
│  - list_child_sessions() → Vec<ChildSessionInfo>             │
│  - replay_events(sub_run_filter?) → EventStream               │
└─────────────────────────────────────────────────────────────┘
```

### 前端数据结构

```typescript
// 会话树节点
interface SessionNode {
  session: SessionMeta;
  subRuns: Map<string, SubRunNode>;  // sub_run_id → SubRunNode
  parent: SessionNode | null;
}

// 子执行节点（SharedSession 模式）
interface SubRunNode {
  subRunId: string;
  agentProfile: string;
  startEvent: SubRunStartMessage;
  finishEvent: SubRunFinishMessage | null;
  messageRange: [number, number];  // 在主会话消息中的索引范围
}

// 会话树
interface SessionTree {
  roots: SessionNode[];
  lookup: Map<string, SessionNode>;  // sessionId → SessionNode
  subRunLookup: Map<string, SessionNode>;  // sub_run_id → 父 SessionNode
}

// 导航状态
interface NavigationState {
  // 当前查看的路径
  // 例：["root-123", "subrun-abc"] 表示正在查看 root-123 中的 subrun-abc
  path: Array<{
    type: 'session' | 'sub-run';
    id: string;
    title: string;
  }>;
  
  // 当前显示的消息
  messages: Message[];
  
  // 当前活跃的 SSE 连接
  activeConnection: {
    sessionId: string;
    subRunFilter: string | null;
  } | null;
}
```

### 后端数据结构

```rust
// 会话树响应
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionTreeResponse {
    pub roots: Vec<SessionTreeNode>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionTreeNode {
    pub session_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    
    // 父子关系
    pub parent_id: Option<String>,
    pub sub_runs: Vec<SubRunInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubRunInfo {
    pub sub_run_id: String,
    pub agent_profile: String,
    pub storage_mode: SubRunStorageMode,
    
    // 独立会话信息
    pub child_session_id: Option<String>,
    
    // 状态
    pub status: SubRunOutcome,
    
    // 时间范围
    pub started_at: String,
    pub finished_at: Option<String>,
    
    // SharedSession 模式下的消息范围
    pub message_range: Option<MessageRange>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageRange {
    pub start_index: usize,
    pub end_index: usize,  // 不包含
}
```

---

## 实施计划

### 阶段 1：后端 API 扩展（2-3 天）

#### 1.1 添加子会话查询 API

**文件**：`crates/server/src/http/routes/sessions/children.rs`

```rust
use axum::{Json, Path, State};
use crate::{ApiError, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildSessionDto {
    pub sub_run_id: String,
    pub agent_profile: String,
    pub storage_mode: SubRunStorageMode,
    pub child_session_id: Option<String>,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// GET /api/sessions/{id}/children
///
/// 返回指定会话的所有子执行信息
pub async fn list_child_sessions(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChildSessionDto>>, ApiError> {
    // 1. 从 agent_control 查询该 session 的所有 sub_run
    let sub_runs = state.service.agent_control.list_sub_runs(&session_id).await?;
    
    // 2. 对每个 sub_run，查询其状态和元数据
    let children: Vec<ChildSessionDto> = sub_runs
        .into_iter()
        .map(|handle| {
            let status = handle.status.as_str().to_string();
            ChildSessionDto {
                sub_run_id: handle.sub_run_id,
                agent_profile: handle.agent_profile,
                storage_mode: handle.storage_mode,
                child_session_id: handle.child_session_id,
                status,
                started_at: "".to_string(),  // 从事件中查询
                finished_at: None,
            }
        })
        .collect();
    
    Ok(Json(children))
}
```

**依赖**：需要在 `AgentControl` 中添加 `list_sub_runs` 方法

```rust
// crates/runtime-agent-control/src/lib.rs

impl AgentControl {
    /// 列出指定 session 的所有子执行
    pub async fn list_sub_runs(
        &self,
        session_id: &str,
    ) -> Result<Vec<SubRunHandle>, AstrError> {
        let registry = self.registry.read().await;
        Ok(registry
            .list_by_session(session_id)
            .into_iter()
            .filter(|handle| {
                // 只返回该 session 的直接子执行，不包括孙级
                handle.parent_session_id.as_ref()
                    .map(|parent_id| parent_id == session_id)
                    .unwrap_or(false)
                    || handle.parent_turn_id.is_some() && handle.session_id == session_id
            })
            .collect())
    }
}
```

#### 1.2 添加会话树查询 API

**文件**：`crates/server/src/http/routes/sessions/tree.rs`

```rust
use axum::{Json, State};
use crate::{ApiError, AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionTreeDto {
    pub roots: Vec<SessionTreeNode>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionTreeNode {
    pub session_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub parent_id: Option<String>,
    pub sub_runs: Vec<SubRunInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubRunInfo {
    pub sub_run_id: String,
    pub agent_profile: String,
    pub storage_mode: String,
    pub child_session_id: Option<String>,
    pub status: String,
}

/// GET /api/sessions/tree
///
/// 返回完整的会话树结构
pub async fn get_session_tree(
    State(state): State<AppState>,
) -> Result<Json<SessionTreeDto>, ApiError> {
    // 1. 获取所有会话元数据
    let sessions = state.service.list_sessions().await?;
    
    // 2. 按 parent_id 分组
    let mut lookup: HashMap<String, SessionTreeNode> = sessions
        .into_iter()
        .map(|meta| {
            let id = meta.session_id.clone();
            let node = SessionTreeNode {
                session_id: id.clone(),
                title: meta.display_name,
                created_at: meta.created_at,
                updated_at: meta.updated_at,
                parent_id: meta.parent_session_id,
                sub_runs: Vec::new(),  // 稍后填充
            };
            (id, node)
        })
        .collect();
    
    // 3. 构建树结构
    let mut roots: Vec<SessionTreeNode> = Vec::new();
    for (id, mut node) in lookup {
        match &node.parent_id {
            Some(parent_id) => {
                if let Some(parent) = lookup.get_mut(parent_id) {
                    // 添加为子节点
                    parent.sub_runs.push(node);
                } else {
                    // 父节点不存在，作为根节点
                    roots.push(node);
                }
            },
            None => {
                roots.push(node);
            },
        }
    }
    
    // 4. 为每个节点填充 sub_run 信息
    for node in roots.iter_mut() {
        fill_sub_run_info(node, &state).await;
    }
    
    Ok(Json(SessionTreeDto { roots }))
}

async fn fill_sub_run_info(
    node: &mut SessionTreeNode,
    state: &AppState,
) -> Result<(), AstrError> {
    // 查询该 session 的所有 sub_run
    let sub_runs = state.service.agent_control.list_sub_runs(&node.session_id).await?;
    
    node.sub_runs = sub_runs
        .into_iter()
        .map(|handle| SubRunInfo {
            sub_run_id: handle.sub_run_id,
            agent_profile: handle.agent_profile,
            storage_mode: format!("{:?}", handle.storage_mode),
            child_session_id: handle.child_session_id,
            status: format!("{:?}", handle.status),
        })
        .collect();
    
    // 递归处理子节点
    for sub_run in &mut node.sub_runs {
        if let Some(child_id) = &sub_run.child_session_id {
            // TODO: 加载子会话树
        }
    }
    
    Ok(())
}
```

#### 1.3 SSE 事件过滤

**文件**：`crates/server/src/http/routes/sessions/stream.rs`

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventsQuery {
    after_event_id: Option<String>,
    token: Option<String>,
    // 新增：过滤参数
    #[serde(default)]
    sub_run: Option<String>,
}

pub(crate) async fn session_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_auth(&state, &headers, query.token.as_deref())?;
    
    let sub_run_filter = query.sub_run;
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.after_event_id);
    
    let mut replay = state
        .service
        .replay(&session_id, last_event_id.as_deref())
        .await
        .map_err(ApiError::from)?;
    
    let service = state.service.clone();
    let session_id_for_stream = session_id.clone();

    let event_stream = stream! {
        // 1. 回放历史事件（应用过滤）
        for record in replay.history {
            if should_emit_event(&record.event, &session_id_for_stream, &sub_run_filter) {
                yield Ok::<Event, Infallible>(to_sse_event(record));
            }
        }

        // 2. 实时事件（应用过滤）
        loop {
            match replay.receiver.recv().await {
                Ok(record) => {
                    if should_emit_event(&record.event, &session_id_for_stream, &sub_run_filter) {
                        yield Ok::<Event, Infallible>(to_sse_event(record));
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::Error::Closed) => break,
            }
        }
    };

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

/// 判断事件是否应该发送
fn should_emit_event(
    event: &astrcode_core::AgentEvent,
    session_id: &str,
    sub_run_filter: &Option<String>,
) -> bool {
    // 没有过滤器 → 发送所有事件
    let sub_run_id = match sub_run_filter {
        None => return true,
        Some(id) => id,
    };

    // 有过滤器 → 只发送匹配的事件
    match event {
        // 有 agent 上下文的事件
        astrcode_core::AgentEvent::UserMessage { agent, .. }
        | astrcode_core::AgentEvent::Assistant { agent, .. }
        | astrcode_core::AgentEvent::ToolCall { agent, .. }
        | astrcode_core::AgentEvent::TurnDone { agent } => {
            agent.sub_run_id.as_deref() == Some(sub_run_id)
        },
        
        // SubRun 事件
        astrcode_core::AgentEvent::SubRunStarted { agent, .. }
        | astrcode_core::AgentEvent::SubRunFinished { agent, .. } => {
            agent.sub_run_id.as_deref() == Some(sub_run_id)
        },
        
        // 其他事件（compact、error 等）属于整个会话，不发送
        _ => false,
    }
}
```

#### 1.4 注册新路由

**文件**：`crates/server/src/http/routes/mod.rs`

```rust
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // ... 现有路由
        .route("/api/sessions/tree", get(sessions::tree::get_session_tree))
        .route("/api/sessions/:id/children", get(sessions::children::list_child_sessions))
}
```

---

### 阶段 2：后端核心逻辑（1-2 天）

#### 2.1 AgentControl 扩展

**文件**：`crates/runtime-agent-control/src/lib.rs`

```rust
impl AgentControl {
    /// 列出指定 session 的所有子执行
    pub async fn list_sub_runs(
        &self,
        session_id: &str,
    ) -> Result<Vec<SubRunHandle>, AstrError> {
        let registry = self.registry.read().await;
        Ok(registry
            .all_handles()
            .into_iter()
            .filter(|handle| {
                // 匹配 session_id
                &handle.session_id == session_id
            })
            .collect())
    }
}
```

**文件**：`crates/runtime-agent-control/src/registry.rs`

```rust
impl AgentRegistry {
    /// 获取所有注册的 handle
    pub fn all_handles(&self) -> Vec<SubRunHandle> {
        self.handles
            .read()
            .values()
            .map(|entry| entry.handle.clone())
            .collect()
    }
    
    /// 列出指定 session 的所有 handle
    pub fn list_by_session(&self, session_id: &str) -> Vec<SubRunHandle> {
        self.handles
            .read()
            .values()
            .filter(|entry| entry.handle.session_id == session_id)
            .map(|entry| entry.handle.clone())
            .collect()
    }
}
```

#### 2.2 RuntimeService 扩展

**文件**：`crates/runtime/src/service/mod.rs`

```rust
impl RuntimeService {
    /// 获取会话树
    pub async fn get_session_tree(&self) -> Result<Vec<SessionTreeNode>, AstrError> {
        let sessions = self.session_manager.list_sessions().await?;
        let mut lookup: HashMap<String, SessionTreeNode> = sessions
            .into_iter()
            .map(|meta| {
                (meta.session_id.clone(), SessionTreeNode {
                    session_id: meta.session_id,
                    title: meta.display_name,
                    created_at: meta.created_at,
                    updated_at: meta.updated_at,
                    parent_id: meta.parent_session_id,
                    sub_runs: Vec::new(),
                })
            })
            .collect();
        
        let mut roots = Vec::new();
        for (_, node) in lookup {
            match &node.parent_id {
                Some(parent_id) => {
                    lookup.entry(parent_id.clone())
                        .or_insert_with(|| default_node(parent_id.clone()))
                        .sub_runs.push(node);
                },
                None => roots.push(node),
            }
        }
        
        Ok(roots)
    }
}

fn default_node(id: String) -> SessionTreeNode {
    SessionTreeNode {
        session_id: id,
        title: "Unknown".to_string(),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        parent_id: None,
        sub_runs: Vec::new(),
    }
}
```

---

### 阶段 3：前端类型定义（半天）

**文件**：`frontend/src/types.ts`

```typescript
// ========== 会话树相关 ==========

export interface SessionTreeNode {
  sessionId: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  parentId?: string | null;
  subRuns: SubRunInfo[];
}

export interface SubRunInfo {
  subRunId: string;
  agentProfile: string;
  storageMode: 'SharedSession' | 'IndependentSession';
  childSessionId?: string | null;
  status: 'Running' | 'Completed' | 'Failed' | 'Aborted' | 'TokenExceeded';
}

// API 响应
export interface SessionTreeResponse {
  roots: SessionTreeNode[];
}

export interface ChildSessionsResponse {
  children: SubRunInfo[];
}

// ========== 导航状态 ==========

export interface NavigationPath {
  type: 'session' | 'sub-run';
  id: string;
  title: string;
  sessionId: string;
  subRunId?: string;
}

export interface NavigationState {
  // 当前导航路径
  path: NavigationPath[];
  
  // 当前显示的消息（已过滤）
  messages: Message[];
  
  // 当前活跃的 SSE 连接
  activeConnection: {
    sessionId: string;
    subRunFilter: string | null;
  } | null;
}

// ========== Redux Action ==========

export type SessionsAction =
  | { type: 'FETCH_SESSION_TREE_SUCCESS'; tree: SessionTreeResponse }
  | { type: 'NAVIGATE_TO_PATH'; path: NavigationPath[] }
  | { type: 'SET_ACTIVE_CONNECTION'; connection: NavigationState['activeConnection'] }
  | { type: 'APPEND_EVENT'; event: AgentEvent };
```

---

### 阶段 4：前端状态管理（1-2 天）

**文件**：`frontend/src/lib/sessionsReducer.ts`

```typescript
const initialState: SessionsState = {
  tree: { roots: [] },
  navigation: {
    path: [],
    messages: [],
    activeConnection: null,
  },
  connections: new Map(),
};

export function sessionsReducer(
  state: SessionsState = initialState,
  action: SessionsAction,
): SessionsState {
  switch (action.type) {
    case 'FETCH_SESSION_TREE_SUCCESS': {
      return {
        ...state,
        tree: buildLookup(action.tree),
      };
    }
    
    case 'NAVIGATE_TO_PATH': {
      const { path } = action;
      
      // 计算需要显示的消息
      const messages = filterMessagesForPath(state, path);
      
      // 更新 SSE 连接
      const activeConnection = deriveActiveConnection(path);
      
      return {
        ...state,
        navigation: {
          path,
          messages,
          activeConnection,
        },
      };
    }
    
    case 'SET_ACTIVE_CONNECTION': {
      return {
        ...state,
        navigation: {
          ...state.navigation,
          activeConnection: action.connection,
        },
      };
    }
    
    case 'APPEND_EVENT': {
      // 追加新事件到消息列表
      const messages = appendEvent(state.navigation.messages, action.event);
      
      return {
        ...state,
        navigation: {
          ...state.navigation,
          messages,
        },
      };
    }
    
    default:
      return state;
  }
}

// 辅助函数
function buildLookup(tree: SessionTreeResponse): SessionsState['tree'] {
  const lookup = new Map<string, SessionTreeNode>();
  
  function traverse(nodes: SessionTreeNode[]) {
    for (const node of nodes) {
      lookup.set(node.sessionId, node);
      // TODO: 处理 subRuns
      traverse(node.subRuns as any);  // 递归处理子节点
    }
  }
  
  traverse(tree.roots);
  
  return {
    roots: tree.roots,
    lookup,
  };
}

function filterMessagesForPath(
  state: SessionsState,
  path: NavigationPath[],
): Message[] {
  if (path.length === 0) return [];
  
  const current = path[path.length - 1];
  
  if (current.type === 'session') {
    // 独立会话：返回所有消息
    return getAllMessagesForSession(current.sessionId);
  } else {
    // sub-run：从主会话中过滤
    const parentMessages = getAllMessagesForSession(current.sessionId);
    return parentMessages.filter(msg =>
      msg.agent?.subRunId === current.subRunId ||
      msg.kind === 'subRunStart' ||
      msg.kind === 'subRunFinish'
    );
  }
}

function deriveActiveConnection(path: NavigationPath[]) {
  if (path.length === 0) return null;
  
  const current = path[path.length - 1];
  
  return {
    sessionId: current.sessionId,
    subRunFilter: current.subRunId ?? null,
  };
}
```

---

### 阶段 5：前端 SSE 管理器（1 天）

**文件**：`frontend/src/lib/sseManager.ts`

```typescript
export class SessionSSEManager {
  private connections = new Map<string, EventSource>();
  private listeners = new Map<string, Set<(event: AgentEvent) => void>>();
  
  /** 为指定会话建立 SSE 连接 */
  connect(
    sessionId: string,
    subRunFilter: string | null,
    onEvent: (event: AgentEvent) => void,
  ) {
    const connectionKey = this.makeConnectionKey(sessionId, subRunFilter);
    
    // 如果已连接，只添加监听器
    if (this.connections.has(connectionKey)) {
      this.addListener(connectionKey, onEvent);
      return;
    }
    
    // 构建查询参数
    const params = new URLSearchParams();
    if (subRunFilter) {
      params.set('subRun', subRunFilter);
    }
    
    const url = `/api/sessions/${sessionId}/events?${params.toString()}`;
    const eventSource = new EventSource(url);
    
    eventSource.onmessage = (e) => {
      const event = parseAgentEvent(e.data);
      
      // 分发给所有监听器
      const listeners = this.listeners.get(connectionKey);
      if (listeners) {
        listeners.forEach(listener => {
          try {
            listener(event);
          } catch (error) {
            console.error('SSE listener error:', error);
          }
        });
      }
    };
    
    eventSource.onerror = (error) => {
      console.error('SSE connection error:', error);
      this.disconnect(sessionId, subRunFilter);
    };
    
    this.connections.set(connectionKey, eventSource);
    this.addListener(connectionKey, onEvent);
  }
  
  /** 断开连接 */
  disconnect(sessionId: string, subRunFilter: string | null) {
    const connectionKey = this.makeConnectionKey(sessionId, subRunFilter);
    
    const eventSource = this.connections.get(connectionKey);
    if (eventSource) {
      eventSource.close();
      this.connections.delete(connectionKey);
    }
    
    this.listeners.delete(connectionKey);
  }
  
  /** 断开某会话的所有连接 */
  disconnectAll(sessionId: string) {
    const toDisconnect: string[] = [];
    
    for (const key of this.connections.keys()) {
      if (key.startsWith(`${sessionId}:`)) {
        toDisconnect.push(key);
      }
    }
    
    for (const key of toDisconnect) {
      const [, sessionId, subRunFilter] = key.split(':');
      this.disconnect(sessionId, subRunFilter || null);
    }
  }
  
  private makeConnectionKey(sessionId: string, subRunFilter: string | null) {
    return `${sessionId}:${subRunFilter ?? ''}`;
  }
  
  private addListener(connectionKey: string, listener: (event: AgentEvent) => void) {
    if (!this.listeners.has(connectionKey)) {
      this.listeners.set(connectionKey, new Set());
    }
    this.listeners.get(connectionKey)!.add(listener);
  }
}

// 单例
export const sseManager = new SessionSSEManager();
```

---

### 阶段 6：前端 UI 组件（2-3 天）

#### 6.1 会话导航组件

**文件**：`frontend/src/components/Session/SessionNav.tsx`

```tsx
import { useSelector } from '../hooks';

export function SessionNav() {
  const { navigation, tree } = useSessionsState();
  
  if (navigation.path.length === 0) {
    return null;
  }
  
  return (
    <nav className="session-nav">
      {navigation.path.map((item, index) => (
        <React.Fragment key={`${item.type}:${item.id}`}>
          {index > 0 && <span className="separator">›</span>}
          
          <button
            className={index === navigation.path.length - 1 ? 'active' : ''}
            onClick={() => navigateToIndex(index)}
          >
            {item.title}
          </button>
        </React.Fragment>
      ))}
    </nav>
  );
}
```

#### 6.2 子会话卡片组件

**文件**：`frontend/src/components/Chat/SubRunCard.tsx`

```tsx
interface SubRunCardProps {
  startEvent: SubRunStartMessage;
  finishEvent?: SubRunFinishMessage;
  onClick: () => void;
}

export function SubRunCard({ startEvent, finishEvent, onClick }: SubRunCardProps) {
  const status = finishEvent?.result.status ?? 'Running';
  
  return (
    <div className="subrun-card" onClick={onClick}>
      <div className="subrun-header">
        <AgentIcon profile={startEvent.agentProfile} />
        <span className="agent-profile">{startEvent.agentProfile}</span>
        <StatusBadge status={status} />
      </div>
      
      {finishEvent ? (
        <div className="subrun-summary">
          <p>{finishEvent.result.summary}</p>
          <div className="subrun-stats">
            <span>Steps: {finishEvent.stepCount}</span>
            <span>Tokens: {finishEvent.estimatedTokens}</span>
          </div>
        </div>
      ) : (
        <div className="subrun-running">
          <Spinner size="small" />
          <span>执行中...</span>
        </div>
      )}
      
      <div className="subrun-footer">
        <button className="view-full-button">
          查看完整会话 →
        </button>
      </div>
    </div>
  );
}
```

#### 6.3 会话切换 Hook

**文件**：`frontend/src/hooks/useSessionNavigation.ts`

```typescript
export function useSessionNavigation() {
  const dispatch = useAppDispatch();
  const { navigation, tree } = useSessionsState();
  
  /** 切换到指定索引的导航路径 */
  const navigateToIndex = (index: number) => {
    const newPath = navigation.path.slice(0, index + 1);
    dispatch({
      type: 'NAVIGATE_TO_PATH',
      path: newPath,
    });
  };
  
  /** 切换到子会话 */
  const navigateToSubRun = (
    parentSessionId: string,
    subRunId: string,
    subRunInfo: SubRunInfo,
  ) => {
    // 构建新的导航路径
    const basePath = navigation.path.filter(
      p => p.sessionId !== parentSessionId
    );
    
    const newPath = [
      ...basePath,
      {
        type: 'sub-run' as const,
        id: subRunId,
        title: `${subRunInfo.agentProfile} Agent`,
        sessionId: parentSessionId,
        subRunId,
      },
    ];
    
    dispatch({
      type: 'NAVIGATE_TO_PATH',
      path: newPath,
    });
  };
  
  /** 切换到独立子会话 */
  const navigateToSession = (sessionId: string) => {
    const node = tree.lookup.get(sessionId);
    if (!node) return;
    
    const newPath = [
      {
        type: 'session' as const,
        id: sessionId,
        title: node.title,
        sessionId,
      },
    ];
    
    dispatch({
      type: 'NAVIGATE_TO_PATH',
      path: newPath,
    });
  };
  
  /** 返回父会话 */
  const navigateToParent = () => {
    if (navigation.path.length > 1) {
      navigateToIndex(navigation.path.length - 2);
    }
  };
  
  return {
    currentPath: navigation.path,
    navigateToIndex,
    navigateToSubRun,
    navigateToSession,
    navigateToParent,
  };
}
```

---

### 阶段 7：前端数据加载（1 天）

**文件**：`frontend/src/lib/sessionLoader.ts`

```typescript
/** 加载会话树 */
export async function loadSessionTree(): Promise<SessionTreeResponse> {
  const response = await fetch('/api/sessions/tree');
  if (!response.ok) {
    throw new Error('Failed to load session tree');
  }
  return response.json();
}

/** 加载子会话列表 */
export async function loadChildSessions(
  sessionId: string,
): Promise<ChildSessionsResponse> {
  const response = await fetch(`/api/sessions/${sessionId}/children`);
  if (!response.ok) {
    throw new Error('Failed to load child sessions');
  }
  return response.json();
}

/** 加载会话消息历史 */
export async function loadSessionMessages(
  sessionId: string,
  subRunFilter?: string,
): Promise<Message[]> {
  const params = new URLSearchParams();
  // 使用较大的 after_event_id 来加载完整历史
  
  const response = await fetch(`/api/sessions/${sessionId}/events`);
  if (!response.ok) {
    throw new Error('Failed to load session messages');
  }
  
  const text = await response.text();
  const events = text
    .split('\n')
    .filter(line => line.trim())
    .map(parseSSMLine)
    .filter(({ event }) => {
      if (!subRunFilter) return true;
      return eventMatchesSubRun(event, subRunFilter);
    });
  
  return convertEventsToMessages(events);
}
```

---

### 阶段 8：集成与测试（1-2 天）

#### 8.1 应用初始化

**文件**：`frontend/src/App.tsx`

```tsx
useEffect(() => {
  // 1. 加载会话树
  loadSessionTree()
    .then(tree => {
      dispatch({
        type: 'FETCH_SESSION_TREE_SUCCESS',
        tree,
      });
    })
    .catch(error => {
      console.error('Failed to load session tree:', error);
    });
}, []);

// 2. 监听导航变化，管理 SSE 连接
useEffect(() => {
  const { activeConnection } = navigation;
  
  if (!activeConnection) return;
  
  // 建立连接
  sseManager.connect(
    activeConnection.sessionId,
    activeConnection.subRunFilter,
    (event) => {
      dispatch({
        type: 'APPEND_EVENT',
        event,
      });
    },
  );
  
  // 清理函数
  return () => {
    sseManager.disconnect(
      activeConnection.sessionId,
      activeConnection.subRunFilter,
    );
  };
}, [navigation.activeConnection]);
```

#### 8.2 测试场景

1. **单会话测试**：创建根会话，发送消息
2. **子会话创建**：调用 runAgent，验证 SubRunStart 事件
3. **子会话切换**：点击子会话卡片，验证导航和消息过滤
4. **SharedSession 测试**：验证事件过滤正确
5. **IndependentSession 测试**：验证独立会话加载和显示
6. **多层嵌套测试**：子 Agent 调用子 Agent
7. **并发测试**：多个子 Agent 并行执行
8. **导航历史测试**：前进/后退功能

---

## 时间线总结

| 阶段 | 任务 | 时间 |
|------|------|------|
| 1 | 后端 API 扩展 | 2-3 天 |
| 2 | 后端核心逻辑 | 1-2 天 |
| 3 | 前端类型定义 | 0.5 天 |
| 4 | 前端状态管理 | 1-2 天 |
| 5 | 前端 SSE 管理器 | 1 天 |
| 6 | 前端 UI 组件 | 2-3 天 |
| 7 | 前端数据加载 | 1 天 |
| 8 | 集成与测试 | 1-2 天 |
| **总计** | | **10-15 天** |

---

## 关键技术点

### 1. SharedSession 模式下的事件归属

问题：SharedSession 中，子 Agent 事件和主会话事件混在一起

解决方案：
- 事件携带 `AgentEventContext.sub_run_id`
- SSE 过滤参数 `?sub_run={id}`
- 前端根据 `sub_run_id` 过滤显示

### 2. 导航状态管理

问题：如何在多层嵌套中正确导航

解决方案：
- 导航栈 `path` 记录完整路径
- 每个节点包含 `sessionId` 和可选的 `subRunId`
- 切换时重新计算消息列表和 SSE 连接

### 3. SSE 连接复用

问题：避免重复建立 SSE 连接

解决方案：
- 使用 `sessionId:subRunFilter` 作为连接键
- 同一连接可以服务多个视图（如果过滤条件兼容）
- 导航切换时智能复用或断开重建

### 4. 消息历史加载

问题：如何高效加载子会话消息

解决方案：
- IndependentSession：直接加载该会话的事件
- SharedSession：加载主会话事件，在内存中过滤

---

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| SSE 连接数过多 | 限制最大并发连接数，自动清理不活跃连接 |
| 事件过滤性能 | 使用索引加速查找，后端预过滤 |
| 状态同步复杂 | 使用 Redux 严格管理状态，避免多处修改 |
| 嵌套层级过深 | 限制最大嵌套深度（如 3 层），UI 展示折叠 |

---

## 后续优化

1. **会话搜索**：在会话树中搜索特定内容
2. **会话导出**：导出子会话为 Markdown
3. **会话分支**：从子会话创建新的根会话
4. **性能优化**：虚拟滚动大量消息
5. **离线支持**：Service Worker 缓存会话数据
