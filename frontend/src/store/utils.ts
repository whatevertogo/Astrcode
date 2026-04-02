//! # Store Utilities
//!
//! Session message conversion, project grouping, and session message replacement helpers.
//! These were previously at the top of App.tsx and made the file harder to navigate.

import type { AssistantMessage, Message, Project, SessionMeta } from '../types';
import { uuid } from '../utils/uuid';
import { snapshotToolStatus } from '../sessionMessages';
import type { SessionMessage as HookSessionMessage } from '../../hooks/useAgent';

function toEpochMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : Date.now();
}

export function convertSessionMessage(message: HookSessionMessage): Message {
  const timestamp =
    message.kind === 'user' || message.kind === 'assistant'
      ? toEpochMs(message.timestamp)
      : Date.now();
  const base = { id: uuid(), timestamp };

  switch (message.kind) {
    case 'user':
      return { ...base, kind: 'user' as const, turnId: message.turnId, text: message.content };
    case 'assistant':
      return {
        ...base,
        kind: 'assistant' as const,
        turnId: message.turnId,
        text: message.content,
        reasoningText: message.reasoningContent,
        streaming: false,
      };
    case 'toolCall':
      return {
        ...base,
        kind: 'toolCall' as const,
        turnId: message.turnId,
        toolCallId: message.toolCallId,
        toolName: message.toolName,
        status: snapshotToolStatus(message.ok),
        args: message.args,
        output: message.output,
        error: message.error,
        metadata: message.metadata,
        durationMs: message.durationMs,
      };
  }
}

function getDirectoryName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || '默认项目';
}

export function groupSessionsByProject(sessionMetas: SessionMeta[]): Project[] {
  const projectMap = new Map<string, { project: Project; maxUpdatedAt: number }>();

  for (const meta of sessionMetas) {
    const projectId = meta.workingDir || '__default_project__';
    const projectName = meta.displayName || getDirectoryName(meta.workingDir);
    const updatedAt = toEpochMs(meta.updatedAt);
    const createdAt = toEpochMs(meta.createdAt);

    let holder = projectMap.get(projectId);
    if (!holder) {
      holder = {
        project: {
          id: projectId,
          name: projectName,
          workingDir: meta.workingDir,
          isExpanded: true,
          sessions: [],
        },
        maxUpdatedAt: updatedAt,
      };
      projectMap.set(projectId, holder);
    } else {
      holder.maxUpdatedAt = Math.max(holder.maxUpdatedAt, updatedAt);
    }

    holder.project.sessions.push({
      id: meta.sessionId,
      projectId,
      title: meta.title || '新会话',
      createdAt,
      updatedAt,
      messages: [],
    });
  }

  const projects = Array.from(projectMap.values());
  projects.sort((a, b) => b.maxUpdatedAt - a.maxUpdatedAt);
  return projects.map((item) => {
    item.project.sessions.sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
    return item.project;
  });
}

export function replaceSessionMessages(
  projects: Project[],
  sessionId: string,
  messages: Message[]
): Project[] {
  return projects.map((project) => ({
    ...project,
    sessions: project.sessions.map((session) =>
      session.id === sessionId ? { ...session, messages } : session
    ),
  }));
}
