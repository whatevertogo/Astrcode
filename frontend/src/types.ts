//! # 类型定义
//!
//! 定义前端使用的所有 TypeScript 类型。

export type Phase = 'idle' | 'thinking' | 'callingTool' | 'streaming' | 'interrupted' | 'done';
export type ToolOutputStream = 'stdout' | 'stderr';
export type CompactTrigger = 'auto' | 'manual';
export type InvocationKind = 'subRun' | 'rootExecution';
// Why: 新写路径已经全部切到 `independentSession`，但前端读侧仍需要识别
// 历史 `sharedSession` 样本并做显式降级展示，不能把旧数据直接类型抹掉。
export type SubRunStorageMode = 'independentSession';
// Why: `legacyDurable` 仍承担“旧数据可识别但不受支持”的投影语义，
// 前端需要它来渲染稳定错误，而不是把样本误当成正常 durable child。
export type SubRunStatusSource = 'live' | 'durable' | 'legacyDurable';
export type SessionEventScope = 'self' | 'subtree' | 'directChildren';
export type UnsupportedLegacyErrorCode = 'unsupported_legacy_shared_history';
export type AgentLifecycle = 'pending' | 'running' | 'idle' | 'terminated';
export type AgentTurnOutcome = 'completed' | 'failed' | 'cancelled' | 'token_exceeded';
// Why: `waiting` 仍保留给 durable child 通知读侧，避免旧事件样本在前端反序列化失败。
export type ChildSessionNotificationKind =
  | 'started'
  | 'progress_summary'
  | 'delivered'
  | 'waiting'
  | 'resumed'
  | 'closed'
  | 'failed';
export type SubRunOutcome = 'running' | 'completed' | 'failed' | 'aborted' | 'token_exceeded';
export type SubRunFailureCode =
  | 'transport'
  | 'provider_http'
  | 'stream_parse'
  | 'interrupted'
  | 'internal';

export interface AgentContext {
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
}

type AgentScoped<T> = T & AgentContext;

export interface ToolCallResultEnvelope {
  toolCallId: string;
  toolName: string;
  ok: boolean;
  output: string;
  error?: string;
  metadata?: unknown;
  durationMs: number;
  truncated?: boolean;
}

export interface PromptMetricsSnapshot {
  stepIndex: number;
  estimatedTokens: number;
  contextWindow: number;
  effectiveWindow: number;
  thresholdTokens: number;
  truncatedToolResults: number;
  providerInputTokens?: number;
  providerOutputTokens?: number;
  cacheCreationInputTokens?: number;
  cacheReadInputTokens?: number;
  providerCacheMetricsSupported?: boolean;
  promptCacheReuseHits?: number;
  promptCacheReuseMisses?: number;
}

export interface ArtifactRef {
  kind: string;
  id: string;
  label: string;
  sessionId?: string;
  storageSeq?: number;
  uri?: string;
}

export interface ResolvedSubagentContextOverrides {
  storageMode: SubRunStorageMode;
  inheritSystemInstructions: boolean;
  inheritProjectInstructions: boolean;
  inheritWorkingDir: boolean;
  inheritPolicyUpperBound: boolean;
  inheritCancelToken: boolean;
  includeCompactSummary: boolean;
  includeRecentTail: boolean;
  includeRecoveryRefs: boolean;
  includeParentFindings: boolean;
}

export interface ResolvedExecutionLimits {
  allowedTools: string[];
  maxSteps?: number;
}

export interface MailboxQueuedEventData {
  turnId?: string | null;
  deliveryId: string;
  fromAgentId: string;
  toAgentId: string;
  message: string;
  queuedAt: string;
  senderLifecycleStatus?: AgentLifecycle;
  senderLastTurnOutcome?: AgentTurnOutcome;
  senderOpenSessionId: string;
  summary?: string;
}

export interface MailboxBatchEventData {
  turnId?: string | null;
  targetAgentId: string;
  batchId: string;
  deliveryIds: string[];
}

export interface MailboxDiscardedEventData {
  turnId?: string | null;
  targetAgentId: string;
  deliveryIds: string[];
}

export interface ExecutionControl {
  maxSteps?: number;
  manualCompact?: boolean;
}

export interface SubRunResult {
  status: SubRunOutcome;
  handoff?: {
    summary: string;
    findings: string[];
    artifacts: ArtifactRef[];
  };
  failure?: {
    code: SubRunFailureCode;
    displayMessage: string;
    technicalMessage: string;
    retryable: boolean;
  };
}

export interface SubRunStatusSnapshot {
  subRunId: string;
  executionId?: string;

  toolCallId?: string;
  source: SubRunStatusSource;
  agentId: string;
  agentProfile: string;
  sessionId: string;
  childSessionId?: string;
  depth: number;
  parentTurnId?: string;
  parentAgentId?: string;
  parentSubRunId?: string;
  storageMode: SubRunStorageMode;
  lifecycle: AgentLifecycle;
  lastTurnOutcome?: AgentTurnOutcome;
  result?: SubRunResult;
  stepCount?: number;
  estimatedTokens?: number;
  resolvedOverrides?: ResolvedSubagentContextOverrides;
  resolvedLimits?: ResolvedExecutionLimits;
}

export type AgentEventPayload =
  | { event: 'sessionStarted'; data: { sessionId: string } }
  | { event: 'userMessage'; data: AgentScoped<{ turnId: string; content: string }> }
  | { event: 'phaseChanged'; data: AgentScoped<{ phase: Phase; turnId?: string | null }> }
  | { event: 'modelDelta'; data: AgentScoped<{ turnId: string; delta: string }> }
  | { event: 'thinkingDelta'; data: AgentScoped<{ turnId: string; delta: string }> }
  | {
      event: 'assistantMessage';
      data: AgentScoped<{ turnId: string; content: string; reasoningContent?: string }>;
    }
  | {
      event: 'toolCallStart';
      data: AgentScoped<{
        turnId: string;
        toolCallId: string;
        toolName: string;
        args: unknown;
      }>;
    }
  | {
      event: 'toolCallDelta';
      data: AgentScoped<{
        turnId: string;
        toolCallId: string;
        toolName: string;
        stream: ToolOutputStream;
        delta: string;
      }>;
    }
  | {
      event: 'toolCallResult';
      data: AgentScoped<{ turnId: string; result: ToolCallResultEnvelope }>;
    }
  | {
      event: 'promptMetrics';
      data: AgentScoped<
        {
          turnId?: string | null;
        } & PromptMetricsSnapshot
      >;
    }
  | {
      event: 'agentMailboxQueued';
      data: AgentScoped<MailboxQueuedEventData>;
    }
  | {
      event: 'agentMailboxBatchStarted';
      data: AgentScoped<MailboxBatchEventData>;
    }
  | {
      event: 'agentMailboxBatchAcked';
      data: AgentScoped<MailboxBatchEventData>;
    }
  | {
      event: 'agentMailboxDiscarded';
      data: AgentScoped<MailboxDiscardedEventData>;
    }
  | {
      event: 'compactApplied';
      data: AgentScoped<{
        turnId?: string | null;
        trigger: CompactTrigger;
        summary: string;
        preservedRecentTurns: number;
      }>;
    }
  | {
      event: 'subRunStarted';
      data: AgentScoped<{
        turnId?: string | null;

        toolCallId?: string;
        resolvedOverrides: ResolvedSubagentContextOverrides;
        resolvedLimits: ResolvedExecutionLimits;
      }>;
    }
  | {
      event: 'subRunFinished';
      data: AgentScoped<{
        turnId?: string | null;

        toolCallId?: string;
        result: SubRunResult;
        stepCount: number;
        estimatedTokens: number;
      }>;
    }
  | {
      event: 'childSessionNotification';
      data: AgentScoped<{
        turnId?: string | null;
        childRef: {
          agentId: string;
          sessionId: string;
          subRunId: string;
          executionId?: string;
          parentAgentId?: string;
          parentSubRunId?: string;
          lineageKind: 'spawn' | 'fork' | 'resume';
          status: AgentLifecycle;
          openSessionId: string;
        };
        kind: ChildSessionNotificationKind;
        summary: string;
        status: AgentLifecycle;
        sourceToolCallId?: string;
        finalReplyExcerpt?: string;
      }>;
    }
  | { event: 'turnDone'; data: AgentScoped<{ turnId: string }> }
  | {
      event: 'error';
      data: AgentScoped<{ turnId?: string | null; code: string; message: string }>;
    };

export type AgentEvent = AgentEventPayload & {
  protocolVersion: number;
};

export type SessionCatalogEventPayload =
  | { event: 'sessionCreated'; data: { sessionId: string } }
  | { event: 'sessionDeleted'; data: { sessionId: string } }
  | { event: 'projectDeleted'; data: { workingDir: string } }
  | {
      event: 'sessionBranched';
      data: { sessionId: string; sourceSessionId: string };
    };

export interface UserMessage {
  id: string;
  kind: 'user';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  text: string;
  timestamp: number;
}

export interface AssistantMessage {
  id: string;
  kind: 'assistant';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  text: string;
  reasoningText?: string;
  streaming: boolean;
  timestamp: number;
}

export type ToolStatus = 'running' | 'ok' | 'fail';

export interface ToolCallMessage {
  id: string;
  kind: 'toolCall';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  toolCallId: string;
  toolName: string;
  status: ToolStatus;
  args: unknown;
  output?: string;
  error?: string;
  metadata?: unknown;
  durationMs?: number;
  truncated?: boolean;
  timestamp: number;
}

export interface CompactMessage {
  id: string;
  kind: 'compact';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  trigger: CompactTrigger;
  summary: string;
  preservedRecentTurns: number;
  timestamp: number;
}

export interface PromptMetricsMessage {
  id: string;
  kind: 'promptMetrics';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  stepIndex: number;
  estimatedTokens: number;
  contextWindow: number;
  effectiveWindow: number;
  thresholdTokens: number;
  truncatedToolResults: number;
  providerInputTokens?: number;
  providerOutputTokens?: number;
  cacheCreationInputTokens?: number;
  cacheReadInputTokens?: number;
  providerCacheMetricsSupported?: boolean;
  promptCacheReuseHits?: number;
  promptCacheReuseMisses?: number;
  timestamp: number;
}

export interface SubRunStartMessage {
  id: string;
  kind: 'subRunStart';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;

  toolCallId?: string;
  resolvedOverrides: ResolvedSubagentContextOverrides;
  resolvedLimits: ResolvedExecutionLimits;
  timestamp: number;
}

export interface SubRunFinishMessage {
  id: string;
  kind: 'subRunFinish';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;

  toolCallId?: string;
  result: SubRunResult;
  stepCount: number;
  estimatedTokens: number;
  timestamp: number;
}

export interface ChildSessionNotificationMessage {
  id: string;
  kind: 'childSessionNotification';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  childRef: {
    agentId: string;
    sessionId: string;
    subRunId: string;
    executionId?: string;
    parentAgentId?: string;
    parentSubRunId?: string;
    lineageKind: 'spawn' | 'fork' | 'resume';
    status: AgentLifecycle;
    openSessionId: string;
  };
  notificationKind: ChildSessionNotificationKind;
  status: AgentLifecycle;
  summary: string;
  sourceToolCallId?: string;
  finalReplyExcerpt?: string;
  timestamp: number;
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | PromptMetricsMessage
  | CompactMessage
  | SubRunStartMessage
  | SubRunFinishMessage
  | ChildSessionNotificationMessage;

export interface ThreadMessageItem {
  kind: 'message';
  message: Message;
}

export interface ThreadSubRunItem {
  kind: 'subRun';
  subRunId: string;
}

export type ThreadItem = ThreadMessageItem | ThreadSubRunItem;

export interface SubRunViewData {
  subRunId: string;
  title: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  bodyMessages: Message[];
  threadItems: ThreadItem[];
  streamFingerprint: string;
  childSessionId?: string;
  parentSubRunId: string | null;
  directChildSubRunIds: string[];
  hasDescriptorLineage: boolean;
}

export interface SubRunThreadTree {
  rootThreadItems: ThreadItem[];
  rootStreamFingerprint: string;
  subRuns: Map<string, SubRunViewData>;
}

export interface Session {
  id: string;
  projectId: string;
  title: string;
  createdAt: number;
  updatedAt?: number;
  parentSessionId?: string;
  messages: Message[];
  subRunThreadTree: SubRunThreadTree;
}

export interface SessionMeta {
  sessionId: string;
  workingDir: string;
  displayName: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  parentSessionId?: string;
  parentStorageSeq?: number;
  phase: Phase;
}

export interface SessionViewSnapshot {
  focusEvents: AgentEventPayload[];
  directChildrenEvents: AgentEventPayload[];
  cursor: string | null;
  phase: Phase;
}

export interface DeleteProjectResult {
  successCount: number;
  failedSessionIds: string[];
}

export interface ProfileView {
  name: string;
  baseUrl: string;
  apiKeyPreview: string;
  models: string[];
}

export interface ConfigView {
  configPath: string;
  activeProfile: string;
  activeModel: string;
  profiles: ProfileView[];
  warning?: string;
}

export interface TestResult {
  success: boolean;
  provider: string;
  model: string;
  error?: string;
}

export interface CurrentModelInfo {
  profileName: string;
  model: string;
  providerKind: string;
}

export interface ModelOption {
  profileName: string;
  model: string;
  providerKind: string;
}

export type ComposerOptionKind = 'command' | 'skill' | 'capability';

export interface ComposerOption {
  kind: ComposerOptionKind;
  id: string;
  title: string;
  description: string;
  insertText: string;
  badges: string[];
  keywords: string[];
}

export interface Project {
  id: string;
  name: string;
  workingDir: string;
  sessions: Session[];
  isExpanded: boolean;
}

export interface AppState {
  projects: Project[];
  activeProjectId: string | null;
  activeSessionId: string | null;
  activeSubRunPath: string[];
  phase: Phase;
}

type AgentActionContext = {
  agentId?: string;
  parentTurnId?: string;
  parentSubRunId?: string;
  agentProfile?: string;
  subRunId?: string;
  executionId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
};

// ────────────────────────────────────────────────────────────
// Reducer action 联合类型（App 和 hooks 之间共享）
// ────────────────────────────────────────────────────────────
export type AtomicAction =
  | { type: 'SET_PHASE'; phase: Phase }
  | { type: 'SET_ACTIVE_PROJECT'; projectId: string }
  | { type: 'PUSH_ACTIVE_SUBRUN'; subRunId: string }
  | { type: 'POP_ACTIVE_SUBRUN' }
  | { type: 'SET_ACTIVE_SUBRUN_PATH'; subRunPath: string[] }
  | { type: 'CLEAR_ACTIVE_SUBRUN_PATH' }
  | { type: 'ADD_PROJECT'; project: Project }
  | { type: 'ADD_SESSION'; projectId: string; session: Session }
  | { type: 'SET_ACTIVE'; projectId: string; sessionId: string }
  | { type: 'TOGGLE_EXPAND'; projectId: string }
  | { type: 'RENAME_PROJECT'; projectId: string; name: string }
  | { type: 'DELETE_PROJECT'; projectId: string }
  | { type: 'RENAME_SESSION'; projectId: string; sessionId: string; title: string }
  | { type: 'DELETE_SESSION'; projectId: string; sessionId: string }
  | { type: 'ADD_MESSAGE'; sessionId: string; message: Message }
  | ({
      type: 'UPSERT_USER_MESSAGE';
      sessionId: string;
      turnId: string;
      content: string;
    } & AgentActionContext)
  | ({
      type: 'APPEND_DELTA';
      sessionId: string;
      turnId: string;
      delta: string;
    } & AgentActionContext)
  | ({
      type: 'APPEND_REASONING_DELTA';
      sessionId: string;
      turnId: string;
      delta: string;
    } & AgentActionContext)
  | ({
      type: 'FINALIZE_ASSISTANT';
      sessionId: string;
      turnId: string;
      content: string;
      reasoningText?: string;
    } & AgentActionContext)
  | { type: 'END_STREAMING'; sessionId: string; turnId: string }
  | ({
      type: 'APPEND_TOOL_CALL_DELTA';
      sessionId: string;
      turnId?: string | null;
      toolCallId: string;
      toolName: string;
      stream: ToolOutputStream;
      delta: string;
    } & AgentActionContext)
  | ({
      type: 'UPDATE_TOOL_CALL';
      sessionId: string;
      turnId?: string | null;
      toolCallId: string;
      toolName: string;
      status: ToolStatus;
      output: string;
      error?: string;
      metadata?: unknown;
      durationMs: number;
      truncated?: boolean;
    } & AgentActionContext)
  | ({
      type: 'UPSERT_PROMPT_METRICS';
      sessionId: string;
      turnId?: string | null;
    } & AgentActionContext &
      PromptMetricsSnapshot)
  | { type: 'SET_WORKING_DIR'; projectId: string; workingDir: string }
  | {
      type: 'INITIALIZE';
      projects: Project[];
      activeProjectId: string | null;
      activeSessionId: string | null;
      activeSubRunPath?: string[];
    }
  | { type: 'REPLACE_SESSION_MESSAGES'; sessionId: string; messages: Message[] }
  | { type: 'ADD_SESSION_BACKEND'; projectId: string; sessionId: string };

export type Action =
  | AtomicAction
  | {
      type: 'APPLY_AGENT_EVENTS_BATCH';
      actions: AtomicAction[];
    };
