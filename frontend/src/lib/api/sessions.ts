//! # Session API Endpoints
//!
//! Session and project CRUD operations.

import type { DeleteProjectResult, SessionMeta } from '../../types';
import { getErrorMessage, request, requestJson, requestRaw } from './client';
// 共享工具函数，消除与 lib/shared/index.ts 的重复定义
import { asRecord, pickStringOrUndefined as pickString, pickOptionalString } from '../shared';
import { normalizeAgentEvent } from '../agentEvent';
import type { AgentEventPayload, Phase } from '../../types';
import { buildSessionEventQueryString } from '../sessionView';
import type { SessionEventFilterQuery } from '../sessionView';

export interface PromptSubmission {
  turnId: string;
  sessionId: string;
  branchedFromSessionId?: string;
}

export interface ChildAgentRef {
  agentId: string;
  sessionId: string;
  subRunId: string;
  executionId?: string;
  parentAgentId?: string;
  lineageKind: 'spawn' | 'fork' | 'resume';
  lineageSnapshot?: LineageSnapshot;
  status: string;
  openable: boolean;
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
  status: string;
  openSessionId: string;
  sourceToolCallId?: string;
  finalReplyExcerpt?: string;
}

export interface ParentChildSummaryList {
  items: ChildSessionNotification[];
}

export interface ChildSessionViewProjection {
  childRef: ChildAgentRef;
  title: string;
  status: string;
  summaryItems: string[];
  latestToolActivity: string[];
  hasFinalReply: boolean;
  childSessionId: string;
  hasDescriptorLineage: boolean;
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

export async function submitPrompt(sessionId: string, text: string): Promise<PromptSubmission> {
  const response = await requestJson<PromptSubmission>(
    `/api/sessions/${encodeURIComponent(sessionId)}/prompts`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text }),
    }
  );
  return response;
}

export async function interruptSession(sessionId: string): Promise<void> {
  await request(`/api/sessions/${encodeURIComponent(sessionId)}/interrupt`, {
    method: 'POST',
  });
}

/// [Legacy] 旧的 subrun-only 取消路径。
/// 新的 child session 模型使用 closeAgent 协作工具通过 CapabilityRouter 执行关闭。
/// 保留此端点用于向后兼容旧客户端。
export async function cancelSubRun(sessionId: string, subRunId: string): Promise<void> {
  await request(
    `/api/v1/sessions/${encodeURIComponent(sessionId)}/subruns/${encodeURIComponent(subRunId)}/cancel`,
    {
      method: 'POST',
    }
  );
}

export async function compactSession(sessionId: string): Promise<void> {
  const response = await requestRaw(`/api/sessions/${encodeURIComponent(sessionId)}/compact`, {
    method: 'POST',
  });
  if (response.status === 409) {
    // 这里把后端的冲突语义提升成稳定用户文案，避免把内部错误串直接暴露到 UI。
    throw new Error('当前会话正在运行，暂不允许手动 compact。');
  }
  if (!response.ok) {
    throw new Error(await getErrorMessage(response));
  }
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
export async function loadParentChildSummaryList(
  sessionId: string
): Promise<ParentChildSummaryList> {
  const payload = await requestJson<ParentChildSummaryList>(
    `/api/sessions/${encodeURIComponent(sessionId)}/children/summary`
  );
  return payload;
}

/// 获取指定子会话的可读视图投影。
/// 投影只包含可消费的摘要信息，不含 raw JSON 或内部 inbox envelope。
export async function loadChildSessionView(
  parentSessionId: string,
  childSessionId: string
): Promise<ChildSessionViewProjection> {
  const payload = await requestJson<{ view: ChildSessionViewProjection }>(
    `/api/sessions/${encodeURIComponent(parentSessionId)}/children/${encodeURIComponent(childSessionId)}/view`
  );
  return payload.view;
}
