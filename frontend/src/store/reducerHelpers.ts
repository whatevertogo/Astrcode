import type { AppState, Project, Session } from '../types';

export function mapProject(
  state: AppState,
  projectId: string,
  fn: (project: Project) => Project
): AppState {
  return {
    ...state,
    projects: state.projects.map((project) => (project.id === projectId ? fn(project) : project)),
  };
}

export function mapSession(
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

export function findAssistantMessageIndex(
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

export function findUserMessageIndex(
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

export function moveUpdatedMessageToTail(
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

export function upsertAssistantTurnMessage(
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

export function findToolCallMessageIndex(
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

export function findPromptMetricsMessageIndex(
  messages: AppState['projects'][number]['sessions'][number]['messages'],
  stepIndex: number,
  turnId?: string | null
): number {
  return messages.findIndex(
    (message) =>
      message.kind === 'promptMetrics' &&
      message.stepIndex === stepIndex &&
      message.turnId === (turnId ?? null)
  );
}
