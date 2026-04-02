//! # Session API Endpoints
//!
//! Session and project CRUD operations.

import type { DeleteProjectResult, SessionMeta } from '../../types';
import { request, requestJson } from './client';

export interface SessionSnapshot {
  kind: 'user' | 'assistant' | 'toolCall';
  turnId?: string | null;
  content?: string;
  timestamp: string;
  reasoningContent?: string;
  toolCallId?: string;
  toolName?: string;
  args?: unknown;
  output?: string;
  error?: string;
  metadata?: unknown;
  ok?: boolean;
  durationMs?: number;
}

export interface PromptSubmission {
  turnId: string;
  sessionId: string;
  branchedFromSessionId?: string;
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

export async function loadSession(sessionId: string): Promise<{
  messages: SessionSnapshot[];
  cursor: string | null;
}> {
  const response = await request(`/api/sessions/${encodeURIComponent(sessionId)}/messages`);
  const payload = (await response.json()) as unknown[];
  const messages = payload.map(normalizeSessionMessage);
  const cursor = response.headers.get('x-session-cursor');
  return { messages, cursor };
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

type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown): UnknownRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as UnknownRecord;
}

function pickString(record: UnknownRecord, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string') {
      return value;
    }
  }
  return undefined;
}

function pickOptionalString(record: UnknownRecord, ...keys: string[]): string | null | undefined {
  for (const key of keys) {
    if (!(key in record)) {
      continue;
    }
    const value = record[key];
    if (value === null || value === undefined) {
      return null;
    }
    if (typeof value === 'string') {
      return value;
    }
    return undefined;
  }
  return undefined;
}

function pickBoolean(record: UnknownRecord, ...keys: string[]): boolean | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'boolean') {
      return value;
    }
  }
  return undefined;
}

function pickNumber(record: UnknownRecord, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}

function normalizeSessionMessage(raw: unknown): SessionSnapshot {
  const message = asRecord(raw);
  if (!message) {
    throw new Error(`invalid session message: ${String(raw)}`);
  }

  const kind = pickString(message, 'kind');
  if (kind === 'user') {
    return {
      kind: 'user',
      turnId: pickOptionalString(message, 'turnId', 'turn_id') ?? null,
      content: pickString(message, 'content') ?? '',
      timestamp: pickString(message, 'timestamp') ?? new Date().toISOString(),
    };
  }

  if (kind === 'assistant') {
    return {
      kind: 'assistant',
      turnId: pickOptionalString(message, 'turnId', 'turn_id') ?? null,
      content: pickString(message, 'content') ?? '',
      timestamp: pickString(message, 'timestamp') ?? new Date().toISOString(),
      reasoningContent: pickString(message, 'reasoningContent', 'reasoning_content'),
    };
  }

  if (kind === 'toolCall') {
    return {
      kind: 'toolCall',
      turnId: pickOptionalString(message, 'turnId', 'turn_id') ?? null,
      toolCallId: pickString(message, 'toolCallId', 'tool_call_id') ?? 'unknown',
      toolName: pickString(message, 'toolName', 'tool_name') ?? '(unknown tool)',
      args: message.args ?? null,
      output: pickString(message, 'output'),
      error: pickString(message, 'error'),
      metadata: message.metadata,
      ok: pickBoolean(message, 'ok'),
      durationMs: pickNumber(message, 'durationMs', 'duration_ms'),
      timestamp: pickString(message, 'timestamp') ?? new Date().toISOString(),
    };
  }

  throw new Error(`unsupported session message kind: ${String(kind)}`);
}
