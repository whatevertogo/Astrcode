//! # 类型定义
//!
//! 定义前端使用的所有 TypeScript 类型。

export type Phase = 'idle' | 'thinking' | 'callingTool' | 'streaming' | 'interrupted' | 'done';
export type ToolOutputStream = 'stdout' | 'stderr';
export type CompactTrigger = 'auto' | 'manual';
export type InvocationKind = 'subRun' | 'rootExecution';
export type SubRunStorageMode = 'sharedSession' | 'independentSession';
export type SubRunOutcome =
  | 'completed'
  | 'aborted'
  | 'token_exceeded'
  | { failed: { error: string } };

export interface AgentContext {
  agentId?: string;
  parentTurnId?: string;
  agentProfile?: string;
  subRunId?: string;
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
  maxSteps?: number;
  tokenBudget?: number;
  allowedTools: string[];
}

export interface SubRunResult {
  summary: string;
  artifacts: ArtifactRef[];
  findings: string[];
  status:
    | 'completed'
    | 'aborted'
    | 'token_exceeded'
    | {
        failed: {
          error: string;
        };
      };
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
        resolvedOverrides: ResolvedSubagentContextOverrides;
        resolvedLimits: ResolvedExecutionLimits;
      }>;
    }
  | {
      event: 'subRunFinished';
      data: AgentScoped<{
        turnId?: string | null;
        result: SubRunResult;
        stepCount: number;
        estimatedTokens: number;
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
  agentProfile?: string;
  subRunId?: string;
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
  agentProfile?: string;
  subRunId?: string;
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
  agentProfile?: string;
  subRunId?: string;
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
  agentProfile?: string;
  subRunId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  trigger: CompactTrigger;
  summary: string;
  preservedRecentTurns: number;
  timestamp: number;
}

export interface SubRunStartMessage {
  id: string;
  kind: 'subRunStart';
  turnId?: string | null;
  agentId?: string;
  parentTurnId?: string;
  agentProfile?: string;
  subRunId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
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
  agentProfile?: string;
  subRunId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
  result: SubRunResult;
  stepCount: number;
  estimatedTokens: number;
  timestamp: number;
}

export type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | CompactMessage
  | SubRunStartMessage
  | SubRunFinishMessage;

export interface Session {
  id: string;
  projectId: string;
  title: string;
  createdAt: number;
  updatedAt?: number;
  messages: Message[];
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
  phase: Phase;
}

type AgentActionContext = {
  agentId?: string;
  parentTurnId?: string;
  agentProfile?: string;
  subRunId?: string;
  invocationKind?: InvocationKind;
  storageMode?: SubRunStorageMode;
  childSessionId?: string;
};

// ────────────────────────────────────────────────────────────
// Reducer action 联合类型（App 和 hooks 之间共享）
// ────────────────────────────────────────────────────────────
export type Action =
  | { type: 'SET_PHASE'; phase: Phase }
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
  | { type: 'SET_WORKING_DIR'; projectId: string; workingDir: string }
  | {
      type: 'INITIALIZE';
      projects: Project[];
      activeProjectId: string | null;
      activeSessionId: string | null;
    }
  | { type: 'REPLACE_SESSION_MESSAGES'; sessionId: string; messages: Message[] }
  | { type: 'ADD_SESSION_BACKEND'; projectId: string; sessionId: string };
