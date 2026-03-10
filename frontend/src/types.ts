export type Phase = 'idle' | 'thinking' | 'callingTool' | 'streaming' | 'interrupted' | 'done';

export interface ToolCallResultEnvelope {
  toolCallId: string;
  toolName: string;
  ok: boolean;
  output: string;
  error?: string;
  metadata?: unknown;
  durationMs: number;
}

export type AgentEventPayload =
  | { event: 'sessionStarted'; data: { sessionId: string } }
  | { event: 'phaseChanged'; data: { phase: Phase; turnId?: string | null } }
  | { event: 'modelDelta'; data: { turnId: string; delta: string } }
  | { event: 'assistantMessage'; data: { turnId: string; content: string } }
  | {
      event: 'toolCallStart';
      data: {
        turnId: string;
        toolCallId: string;
        toolName: string;
        args: unknown;
      };
    }
  | {
      event: 'toolCallResult';
      data: { turnId: string; result: ToolCallResultEnvelope };
    }
  | { event: 'turnDone'; data: { turnId: string } }
  | {
      event: 'error';
      data: { turnId?: string | null; code: string; message: string };
    };

export type AgentEvent = AgentEventPayload & {
  protocolVersion: number;
};

export interface UserMessage {
  id: string;
  kind: 'user';
  text: string;
  timestamp: number;
}

export interface AssistantMessage {
  id: string;
  kind: 'assistant';
  text: string;
  streaming: boolean;
  timestamp: number;
}

export type ToolStatus = 'running' | 'ok' | 'fail';

export interface ToolCallMessage {
  id: string;
  kind: 'toolCall';
  toolCallId: string;
  toolName: string;
  status: ToolStatus;
  args: unknown;
  output?: string;
  error?: string;
  durationMs?: number;
  timestamp: number;
}

export type Message = UserMessage | AssistantMessage | ToolCallMessage;

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

// ────────────────────────────────────────────────────────────
// Reducer action union (shared across App + hooks)
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
  | { type: 'APPEND_DELTA'; sessionId: string; delta: string }
  | { type: 'END_STREAMING'; sessionId: string }
  | {
      type: 'UPDATE_TOOL_CALL';
      sessionId: string;
      toolCallId: string;
      status: ToolStatus;
      output: string;
      error?: string;
      durationMs: number;
    }
  | { type: 'SET_WORKING_DIR'; projectId: string; workingDir: string }
  | {
      type: 'INITIALIZE';
      projects: Project[];
      activeProjectId: string | null;
      activeSessionId: string | null;
    }
  | { type: 'REPLACE_SESSION_MESSAGES'; sessionId: string; messages: Message[] }
  | { type: 'ADD_SESSION_BACKEND'; projectId: string; sessionId: string };
