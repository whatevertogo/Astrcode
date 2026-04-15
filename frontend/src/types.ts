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
export type ParentDeliveryOrigin = 'explicit' | 'fallback';
export type ParentDeliveryTerminalSemantics = 'non_terminal' | 'terminal';
export type ParentDeliveryKind = 'progress' | 'completed' | 'failed' | 'close_request';

export type ParentDelivery =
  | {
      idempotencyKey: string;
      origin: ParentDeliveryOrigin;
      terminalSemantics: ParentDeliveryTerminalSemantics;
      sourceTurnId?: string;
      kind: 'progress';
      payload: {
        message: string;
      };
    }
  | {
      idempotencyKey: string;
      origin: ParentDeliveryOrigin;
      terminalSemantics: ParentDeliveryTerminalSemantics;
      sourceTurnId?: string;
      kind: 'completed';
      payload: {
        message: string;
        findings: string[];
        artifacts: ArtifactRef[];
      };
    }
  | {
      idempotencyKey: string;
      origin: ParentDeliveryOrigin;
      terminalSemantics: ParentDeliveryTerminalSemantics;
      sourceTurnId?: string;
      kind: 'failed';
      payload: {
        message: string;
        code: SubRunFailureCode;
        technicalMessage?: string;
        retryable: boolean;
      };
    }
  | {
      idempotencyKey: string;
      origin: ParentDeliveryOrigin;
      terminalSemantics: ParentDeliveryTerminalSemantics;
      sourceTurnId?: string;
      kind: 'close_request';
      payload: {
        message: string;
        reason?: string;
      };
    };

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

export interface ExecutionControl {
  maxSteps?: number;
  manualCompact?: boolean;
}

export interface SubRunResult {
  status: SubRunOutcome;
  handoff?: {
    findings: string[];
    artifacts: ArtifactRef[];
    delivery?: ParentDelivery;
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

export interface ToolStreamMessage {
  id: string;
  kind: 'toolStream';
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
  stream: ToolOutputStream;
  status: ToolStatus;
  content: string;
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
  sourceToolCallId?: string;
  delivery?: ParentDelivery;
  timestamp: number;
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | ToolStreamMessage
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
  latestNotification?: ChildSessionNotificationMessage;
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

export interface OperationMetricsSnapshot {
  total: number;
  failures: number;
  totalDurationMs: number;
  lastDurationMs: number;
  maxDurationMs: number;
}

export interface ReplayMetricsSnapshot {
  totals: OperationMetricsSnapshot;
  cacheHits: number;
  diskFallbacks: number;
  recoveredEvents: number;
}

export interface SubRunExecutionMetricsSnapshot {
  total: number;
  failures: number;
  completed: number;
  aborted: number;
  tokenExceeded: number;
  independentSessionTotal: number;
  totalDurationMs: number;
  lastDurationMs: number;
  totalSteps: number;
  lastStepCount: number;
  totalEstimatedTokens: number;
  lastEstimatedTokens: number;
}

export interface ExecutionDiagnosticsSnapshot {
  childSpawned: number;
  childStartedPersisted: number;
  childTerminalPersisted: number;
  parentReactivationRequested: number;
  parentReactivationSucceeded: number;
  parentReactivationFailed: number;
  lineageMismatchParentAgent: number;
  lineageMismatchParentSession: number;
  lineageMismatchChildSession: number;
  lineageMismatchDescriptorMissing: number;
  cacheReuseHits: number;
  cacheReuseMisses: number;
  deliveryBufferQueued: number;
  deliveryBufferDequeued: number;
  deliveryBufferWakeRequested: number;
  deliveryBufferWakeSucceeded: number;
  deliveryBufferWakeFailed: number;
}

export interface AgentCollaborationScorecard {
  totalFacts: number;
  spawnAccepted: number;
  spawnRejected: number;
  sendReused: number;
  sendQueued: number;
  sendRejected: number;
  observeCalls: number;
  observeRejected: number;
  observeFollowedByAction: number;
  closeCalls: number;
  closeRejected: number;
  deliveryDelivered: number;
  deliveryConsumed: number;
  deliveryReplayed: number;
  orphanChildCount: number;
  childReuseRatioBps?: number;
  observeToActionRatioBps?: number;
  spawnToDeliveryRatioBps?: number;
  orphanChildRatioBps?: number;
  avgDeliveryLatencyMs?: number;
  maxDeliveryLatencyMs?: number;
}

export interface RuntimeMetricsSnapshot {
  sessionRehydrate: OperationMetricsSnapshot;
  sseCatchUp: ReplayMetricsSnapshot;
  turnExecution: OperationMetricsSnapshot;
  subrunExecution: SubRunExecutionMetricsSnapshot;
  executionDiagnostics: ExecutionDiagnosticsSnapshot;
  agentCollaboration: AgentCollaborationScorecard;
}

export interface RuntimeDebugOverview {
  collectedAt: string;
  spawnRejectionRatioBps?: number;
  metrics: RuntimeMetricsSnapshot;
}

export interface RuntimeDebugTimelineSample {
  collectedAt: string;
  spawnRejectionRatioBps?: number;
  observeToActionRatioBps?: number;
  childReuseRatioBps?: number;
}

export interface RuntimeDebugTimeline {
  windowStartedAt: string;
  windowEndedAt: string;
  samples: RuntimeDebugTimelineSample[];
}

export type SessionDebugTraceItemKind =
  | 'toolCall'
  | 'toolResult'
  | 'promptMetrics'
  | 'subRunStarted'
  | 'subRunFinished'
  | 'childNotification'
  | 'collaborationFact'
  | 'mailboxQueued'
  | 'mailboxBatchStarted'
  | 'mailboxBatchAcked'
  | 'mailboxDiscarded'
  | 'turnDone'
  | 'error';

export interface SessionDebugTraceItem {
  id: string;
  storageSeq: number;
  turnId?: string;
  recordedAt?: string;
  kind: SessionDebugTraceItemKind;
  title: string;
  summary: string;
  agentId?: string;
  subRunId?: string;
  childAgentId?: string;
  deliveryId?: string;
  toolCallId?: string;
  toolName?: string;
  lifecycle?: AgentLifecycle;
  lastTurnOutcome?: AgentTurnOutcome;
}

export interface SessionDebugTrace {
  sessionId: string;
  title: string;
  phase: Phase;
  parentSessionId?: string;
  items: SessionDebugTraceItem[];
}

export type DebugAgentNodeKind = 'sessionRoot' | 'childAgent';

export interface SessionDebugAgentNode {
  nodeId: string;
  kind: DebugAgentNodeKind;
  title: string;
  agentId: string;
  sessionId: string;
  childSessionId?: string;
  subRunId?: string;
  parentAgentId?: string;
  parentSessionId?: string;
  depth: number;
  lifecycle: AgentLifecycle;
  lastTurnOutcome?: AgentTurnOutcome;
  statusSource?: string;
  lineageKind?: string;
}

export interface SessionDebugAgents {
  sessionId: string;
  title: string;
  nodes: SessionDebugAgentNode[];
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
  | { type: 'SET_ACTIVE'; projectId: string; sessionId: string }
  | { type: 'TOGGLE_EXPAND'; projectId: string }
  | { type: 'ADD_MESSAGE'; sessionId: string; message: Message }
  | {
      type: 'INITIALIZE';
      projects: Project[];
      activeProjectId: string | null;
      activeSessionId: string | null;
      activeSubRunPath?: string[];
    }
  | {
      type: 'REPLACE_SESSION_MESSAGES';
      sessionId: string;
      messages: Message[];
      subRunThreadTree: SubRunThreadTree;
    };

export type Action = AtomicAction;
