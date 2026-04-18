//! # Session API Endpoints
//!
//! Session and project CRUD operations.

import type {
  AgentLifecycle,
  DeleteProjectResult,
  ExecutionControl,
  SessionMeta,
} from '../../types';
import { request, requestJson } from './client';

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
  status: AgentLifecycle;
  sourceToolCallId?: string;
  delivery?: import('../../types').ParentDelivery;
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

export async function forkSession(
  sessionId: string,
  options?: { turnId?: string; storageSeq?: number }
): Promise<SessionMeta> {
  return requestJson<SessionMeta>(`/api/sessions/${encodeURIComponent(sessionId)}/fork`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      turnId: options?.turnId,
      storageSeq: options?.storageSeq,
    }),
  });
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
  control?: ExecutionControl,
  instructions?: string
): Promise<CompactSessionAcceptance> {
  return requestJson<CompactSessionAcceptance>(
    `/api/sessions/${encodeURIComponent(sessionId)}/compact`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ control, instructions }),
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

/// 获取父会话的子会话 delivery 列表。
/// 父视图只消费 typed delivery，不消费子会话原始事件流。
