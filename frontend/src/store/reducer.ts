//! # Reducer + Action Handlers
//!
//! Central state management for the app.
//! Extracted from App.tsx to improve readability and testability.

import type { AppState, Action, Session, Project } from '../types';
import { uuid } from '../utils/uuid';

// ─── Helpers (extracted from App.tsx) ─────────────────────────────────────────

function mapProject(
  state: AppState,
  projectId: string,
  fn: (project: Project) => Project
): AppState {
  return {
    ...state,
    projects: state.projects.map((project) => (project.id === projectId ? fn(project) : project)),
  };
}

function mapSession(
  state: AppState,
  sessionId: string,
  fn: (session: Session) => Session
): AppState {
  return {
    ...state,
    projects: state.projects.map((project) => ({
      ...project,
      sessions: project.sessions.map((session) =>
        session.id === sessionId ? fn(session) : session
      ),
    })),
  };
}

function findAssistantMessageIndex(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  turnId: string
): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.turnId === turnId) {
      if (message.kind === 'assistant') {
        return index;
      }
      if (message.kind === 'toolCall') {
        // If we encouter a tool call for the same turn before finding an assistant message
        // (since we iterate backwards), it means the contiguous assistant stream is broken.
        // Returning -1 forces creating a new AssistantMessage to appear chronologically AFTER the tool.
        return -1;
      }
    }
  }
  return -1;
}

function findUserMessageIndex(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  turnId: string
): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.kind === 'user' && message.turnId === turnId) {
      return index;
    }
  }
  return -1;
}

function moveUpdatedMessageToTail(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  targetIndex: number,
  updatedMessage: AppState['projects'][number]['sessions'][number]['messages'][number]
): AppState['projects'][number]['sessions'][number]['messages'] {
  if (targetIndex < 0) {
    return messages;
  }

  if (targetIndex === messages.length - 1) {
    return [...messages.slice(0, -1), updatedMessage];
  }

  return [...messages.slice(0, targetIndex), ...messages.slice(targetIndex + 1), updatedMessage];
}

function upsertAssistantTurnMessage(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  turnId: string,
  createMessage: () => AppState['projects'][number]['sessions'][number]['messages'][number] & {
    kind: 'assistant';
  },
  updateMessage: (
    message: AppState['projects'][number]['sessions'][number]['messages'][number] & {
      kind: 'assistant';
    }
  ) => AppState['projects'][number]['sessions'][number]['messages'][number] & { kind: 'assistant' }
): AppState['projects'][number]['sessions'][number]['messages'] {
  const targetIndex = findAssistantMessageIndex(messages, turnId);
  if (targetIndex < 0) {
    return [...messages, createMessage()];
  }

  const target = messages[targetIndex];
  if (target.kind !== 'assistant') {
    return [...messages, createMessage()];
  }

  return moveUpdatedMessageToTail(messages, targetIndex, updateMessage(target));
}

function findToolCallMessageIndex(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  toolCallId: string,
  toolName: string,
  turnId?: string | null,
  requireRunning = false
): number {
  const exactMatchIndex = messages.findIndex(
    (message) => message.kind === 'toolCall' && message.toolCallId === toolCallId
  );
  if (exactMatchIndex >= 0) {
    return exactMatchIndex;
  }

  const fallbackCandidates: number[] = [];
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    const turnMatches =
      turnId === null || turnId === undefined
        ? message.turnId === null || message.turnId === undefined
        : message.turnId === turnId;
    if (
      message.kind === 'toolCall' &&
      (!requireRunning || message.status === 'running') &&
      message.toolName === toolName &&
      turnMatches
    ) {
      fallbackCandidates.push(index);
    }
  }

  return fallbackCandidates.length === 1 ? fallbackCandidates[0] : -1;
}

// ─── Re-exports used by App.tsx ───────────────────────────────────────────────

export {
  findAssistantMessageIndex,
  findUserMessageIndex,
  moveUpdatedMessageToTail,
  upsertAssistantTurnMessage,
  findToolCallMessageIndex,
};

// ─── Reducer ──────────────────────────────────────────────────────────────────

import { appendToolDeltaMetadata, mergeToolMetadata } from '../lib/toolDisplay';

export function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'SET_PHASE':
      if (state.phase === action.phase) {
        return state;
      }
      return { ...state, phase: action.phase };

    case 'SET_ACTIVE':
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
      };

    case 'ADD_PROJECT':
      return {
        ...state,
        projects: [action.project, ...state.projects],
        activeProjectId: action.project.id,
        activeSessionId: action.project.sessions[0]?.id ?? null,
      };

    case 'ADD_SESSION':
      return {
        ...mapProject(state, action.projectId, (project) => ({
          ...project,
          sessions: [action.session, ...project.sessions],
        })),
        activeProjectId: action.projectId,
        activeSessionId: action.session.id,
      };

    case 'TOGGLE_EXPAND':
      return mapProject(state, action.projectId, (project) => ({
        ...project,
        isExpanded: !project.isExpanded,
      }));

    case 'DELETE_PROJECT': {
      const projects = state.projects.filter((project) => project.id !== action.projectId);
      let activeProjectId = state.activeProjectId;
      let activeSessionId = state.activeSessionId;
      if (activeProjectId === action.projectId) {
        activeProjectId = projects[0]?.id ?? null;
        activeSessionId = projects[0]?.sessions[0]?.id ?? null;
      }
      return { ...state, projects, activeProjectId, activeSessionId };
    }

    case 'DELETE_SESSION': {
      const nextState = mapProject(state, action.projectId, (project) => ({
        ...project,
        sessions: project.sessions.filter((session) => session.id !== action.sessionId),
      }));
      let activeSessionId = nextState.activeSessionId;
      let activeProjectId = nextState.activeProjectId;
      if (state.activeSessionId === action.sessionId) {
        const project = nextState.projects.find((item) => item.id === action.projectId);
        activeSessionId = project?.sessions[0]?.id ?? null;
        activeProjectId = project?.id ?? nextState.projects[0]?.id ?? null;
      }
      return { ...nextState, activeProjectId, activeSessionId };
    }

    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        let title = session.title;
        if (
          action.message.kind === 'user' &&
          session.messages.filter((message) => message.kind === 'user').length === 0
        ) {
          title = action.message.text.slice(0, 20) || '新会话';
        }
        return { ...session, title, messages: [...session.messages, action.message] };
      });

    case 'UPSERT_USER_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findUserMessageIndex(session.messages, action.turnId);
        const userMessage = {
          id:
            targetIndex >= 0 && session.messages[targetIndex]?.kind === 'user'
              ? session.messages[targetIndex].id
              : uuid(),
          kind: 'user' as const,
          turnId: action.turnId,
          agentId: action.agentId,
          parentTurnId: action.parentTurnId,
          agentProfile: action.agentProfile,
          subRunId: action.subRunId,
          invocationKind: action.invocationKind,
          storageMode: action.storageMode,
          childSessionId: action.childSessionId,
          text: action.content,
          timestamp:
            targetIndex >= 0 && session.messages[targetIndex]?.kind === 'user'
              ? session.messages[targetIndex].timestamp
              : Date.now(),
        };

        let title = session.title;
        if (session.messages.filter((message) => message.kind === 'user').length === 0) {
          title = action.content.slice(0, 20) || '新会话';
        }

        if (targetIndex < 0) {
          return {
            ...session,
            title,
            messages: [...session.messages, userMessage],
          };
        }

        return {
          ...session,
          title,
          messages: moveUpdatedMessageToTail(session.messages, targetIndex, userMessage),
        };
      });

    case 'APPEND_DELTA':
      return mapSession(state, action.sessionId, (session) => {
        return {
          ...session,
          messages: upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: action.delta,
              reasoningText: '',
              streaming: true,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              text: message.text + action.delta,
              streaming: true,
            })
          ),
        };
      });

    case 'APPEND_REASONING_DELTA':
      return mapSession(state, action.sessionId, (session) => {
        return {
          ...session,
          messages: upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: '',
              reasoningText: action.delta,
              streaming: true,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              reasoningText: `${message.reasoningText ?? ''}${action.delta}`,
              streaming: true,
            })
          ),
        };
      });

    case 'FINALIZE_ASSISTANT':
      return mapSession(state, action.sessionId, (session) => {
        return {
          ...session,
          messages: upsertAssistantTurnMessage(
            session.messages,
            action.turnId,
            () => ({
              id: uuid(),
              kind: 'assistant',
              turnId: action.turnId,
              agentId: action.agentId,
              parentTurnId: action.parentTurnId,
              agentProfile: action.agentProfile,
              subRunId: action.subRunId,
              invocationKind: action.invocationKind,
              storageMode: action.storageMode,
              childSessionId: action.childSessionId,
              text: action.content,
              reasoningText: action.reasoningText,
              streaming: false,
              timestamp: Date.now(),
            }),
            (message) => ({
              ...message,
              turnId: action.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              text: action.content,
              reasoningText: action.reasoningText ?? message.reasoningText,
              streaming: false,
            })
          ),
        };
      });

    case 'END_STREAMING':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findAssistantMessageIndex(session.messages, action.turnId);
        if (targetIndex < 0) {
          return session;
        }

        const target = session.messages[targetIndex];
        if (target.kind !== 'assistant') {
          return session;
        }

        return {
          ...session,
          messages: moveUpdatedMessageToTail(session.messages, targetIndex, {
            ...target,
            streaming: false,
          }),
        };
      });

    case 'APPEND_TOOL_CALL_DELTA':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findToolCallMessageIndex(
          session.messages,
          action.toolCallId,
          action.toolName,
          action.turnId,
          false
        );

        if (targetIndex < 0) {
          return {
            ...session,
            messages: [
              ...session.messages,
              {
                id: uuid(),
                kind: 'toolCall',
                turnId: action.turnId,
                agentId: action.agentId,
                parentTurnId: action.parentTurnId,
                agentProfile: action.agentProfile,
                subRunId: action.subRunId,
                invocationKind: action.invocationKind,
                storageMode: action.storageMode,
                childSessionId: action.childSessionId,
                toolCallId: action.toolCallId,
                toolName: action.toolName,
                status: 'running',
                args: null,
                output: action.delta,
                metadata: appendToolDeltaMetadata(
                  undefined,
                  action.toolName,
                  null,
                  action.stream,
                  action.delta
                ),
                timestamp: Date.now(),
              },
            ],
          };
        }

        return {
          ...session,
          messages: session.messages.map((message, index) => {
            if (index !== targetIndex || message.kind !== 'toolCall') {
              return message;
            }
            return {
              ...message,
              turnId: action.turnId ?? message.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              toolCallId: action.toolCallId,
              toolName: action.toolName,
              output: `${message.output ?? ''}${action.delta}`,
              metadata: appendToolDeltaMetadata(
                message.metadata,
                action.toolName,
                message.args,
                action.stream,
                action.delta
              ),
            };
          }),
        };
      });

    case 'UPDATE_TOOL_CALL':
      return mapSession(state, action.sessionId, (session) => {
        const targetIndex = findToolCallMessageIndex(
          session.messages,
          action.toolCallId,
          action.toolName,
          action.turnId,
          true
        );

        if (targetIndex < 0) {
          return {
            ...session,
            messages: [
              ...session.messages,
              {
                id: uuid(),
                kind: 'toolCall',
                turnId: action.turnId,
                agentId: action.agentId,
                parentTurnId: action.parentTurnId,
                agentProfile: action.agentProfile,
                subRunId: action.subRunId,
                invocationKind: action.invocationKind,
                storageMode: action.storageMode,
                childSessionId: action.childSessionId,
                toolCallId: action.toolCallId,
                toolName: action.toolName,
                status: action.status,
                args: null,
                output: action.output,
                error: action.error,
                metadata: action.metadata,
                durationMs: action.durationMs,
                truncated: action.truncated,
                timestamp: Date.now(),
              },
            ],
          };
        }

        return {
          ...session,
          messages: session.messages.map((message, index) => {
            if (index !== targetIndex || message.kind !== 'toolCall') {
              return message;
            }
            const isShellTool = message.toolName === 'shell' || action.toolName === 'shell';
            return {
              ...message,
              turnId: action.turnId ?? message.turnId,
              agentId: action.agentId ?? message.agentId,
              parentTurnId: action.parentTurnId ?? message.parentTurnId,
              agentProfile: action.agentProfile ?? message.agentProfile,
              subRunId: action.subRunId ?? message.subRunId,
              invocationKind: action.invocationKind ?? message.invocationKind,
              storageMode: action.storageMode ?? message.storageMode,
              childSessionId: action.childSessionId ?? message.childSessionId,
              toolCallId: action.toolCallId,
              toolName: action.toolName,
              status: action.status,
              output: isShellTool && message.output ? message.output : action.output,
              error: action.error,
              metadata: mergeToolMetadata(message.metadata, action.metadata),
              durationMs: action.durationMs,
              truncated: action.truncated,
            };
          }),
        };
      });

    case 'INITIALIZE':
      return {
        ...state,
        projects: action.projects,
        activeProjectId: action.activeProjectId,
        activeSessionId: action.activeSessionId,
      };

    case 'REPLACE_SESSION_MESSAGES':
      return mapSession(state, action.sessionId, (session) => ({
        ...session,
        messages: action.messages,
      }));

    default:
      return state;
  }
}

export function makeInitialState(): AppState {
  return {
    projects: [],
    activeProjectId: null,
    activeSessionId: null,
    phase: 'idle',
  };
}
