//! # Session API Endpoints
//!
//! Session and project CRUD operations.

import type {
  AgentEventPayload,
  AgentLifecycle,
  DeleteProjectResult,
  ExecutionControl,
  Phase,
  SessionMeta,
  SessionViewSnapshot,
} from '../../types';
import { request, requestJson } from './client';
// 共享工具函数，消除与 lib/shared/index.ts 的重复定义
import { asRecord, pickStringOrUndefined as pickString, pickOptionalString } from '../shared';
import { normalizeAgentEvent } from '../agentEvent';
import { buildSessionEventQueryString } from '../sessionView';
import type { SessionEventFilterQuery } from '../sessionView';

export interface PromptSubmission {
  turnId: string;
  sessionId: string;
  branchedFromSessionId?: string;
  acceptedControl?: ExecutionControl;
}

export interface CompactSessionAcceptance {
  accepted: boolean;
  deferred: boolean;
  message: string;
}

export interface ChildAgentRef {
  agentId: string;
  sessionId: string;
  subRunId: string;
  executionId?: string;
  parentAgentId?: string;
  parentSubRunId?: string;
  lineageKind: 'spawn' | 'fork' | 'resume';
  lineageSnapshot?: LineageSnapshot;
  status: AgentLifecycle;
  openSessionId: string;
}

/** 谱系来源快照，fork/resume 时记录来源上下文。 */
export interface LineageSnapshot {
  sourceAgentId: string;
  sourceSessionId: string;
  sourceSubRunId?: string;
}

export interface ChildSessionNotification {
  notificationId: string;
  childRef: ChildAgentRef;
  kind: 'started' | 'progress_summary' | 'delivered' | 'waiting' | 'resumed' | 'closed' | 'failed';
  summary: string;
  status: AgentLifecycle;
  sourceToolCallId?: string;
  finalReplyExcerpt?: string;
}

export async function createSession(workingDir: string): Promise<SessionMeta> {
  return requestJson<SessionMeta>('/api/sessions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ workingDir }),
  });
}

export async function listSessionsWithMeta(): Promise<SessionMeta[]> {
  return requestJson<SessionMeta[]>('/api/sessions');
}

export async function loadSession(
  sessionId: string,
  filter?: SessionEventFilterQuery
): Promise<{
  events: AgentEventPayload[];
  cursor: string | null;
  phase: Phase;
}> {
  const response = await request(
    `/api/sessions/${encodeURIComponent(sessionId)}/history${buildSessionEventQueryString({ filter })}`
  );
  const payload = asRecord((await response.json()) as unknown);
  if (!payload) {
    throw new Error('invalid session history response');
  }

  const eventsRaw = Array.isArray(payload.events) ? payload.events : [];
  const phase = pickString(payload, 'phase');
  if (
    phase !== 'idle' &&
    phase !== 'thinking' &&
    phase !== 'callingTool' &&
    phase !== 'streaming' &&
    phase !== 'interrupted' &&
    phase !== 'done'
  ) {
    throw new Error(`invalid session phase: ${String(phase)}`);
  }

  return {
    events: eventsRaw.map((event) => normalizeAgentEvent(event)),
    cursor: pickOptionalString(payload, 'cursor') ?? null,
    phase,
  };
}

export async function loadSessionView(
  sessionId: string,
  filter?: SessionEventFilterQuery
): Promise<SessionViewSnapshot> {
  const response = await request(
    `/api/sessions/${encodeURIComponent(sessionId)}/view${buildSessionEventQueryString({ filter })}`
  );
  const payload = asRecord((await response.json()) as unknown);
  if (!payload) {
    throw new Error('invalid session view response');
  }

  const phase = pickString(payload, 'phase');
  if (
    phase !== 'idle' &&
    phase !== 'thinking' &&
    phase !== 'callingTool' &&
    phase !== 'streaming' &&
    phase !== 'interrupted' &&
    phase !== 'done'
  ) {
    throw new Error(`invalid session phase: ${String(phase)}`);
  }

  const normalizeEvents = (value: unknown): AgentEventPayload[] =>
    Array.isArray(value) ? value.map((event) => normalizeAgentEvent(event)) : [];

  return {
    focusEvents: normalizeEvents(payload.focusEvents),
    directChildrenEvents: normalizeEvents(payload.directChildrenEvents),
    cursor: pickOptionalString(payload, 'cursor') ?? null,
    phase: phase as Phase,
  };
}

export async function submitPrompt(
  sessionId: string,
  text: string,
  control?: ExecutionControl
): Promise<PromptSubmission> {
  const response = await requestJson<PromptSubmission>(
    `/api/sessions/${encodeURIComponent(sessionId)}/prompts`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text, control }),
    }
  );
  return response;
}

export async function interruptSession(sessionId: string): Promise<void> {
  await request(`/api/sessions/${encodeURIComponent(sessionId)}/interrupt`, {
    method: 'POST',
  });
}

/// 关闭指定 agent 及其子树。
///
/// 按 agent_id 定位，始终级联关闭。
export async function closeChildAgent(
  sessionId: string,
  agentId: string
): Promise<{ closedAgentIds: string[] }> {
  return requestJson<{ closedAgentIds: string[] }>(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/agents/${encodeURIComponent(agentId)}/close`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    }
  );
}

export async function compactSession(
  sessionId: string,
  control?: ExecutionControl
): Promise<CompactSessionAcceptance> {
  return requestJson<CompactSessionAcceptance>(
    `/api/sessions/${encodeURIComponent(sessionId)}/compact`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ control }),
    }
  );
}

export async function deleteSession(sessionId: string): Promise<void> {
  await request(`/api/sessions/${encodeURIComponent(sessionId)}`, {
    method: 'DELETE',
  });
}

export async function deleteProject(workingDir: string): Promise<DeleteProjectResult> {
  return requestJson<DeleteProjectResult>(
    `/api/projects?workingDir=${encodeURIComponent(workingDir)}`,
    {
      method: 'DELETE',
    }
  );
}

/// 获取父会话的子会话摘要列表。
/// 父视图只消费摘要，不消费子会话原始事件流。
