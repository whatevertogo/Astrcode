import type { AgentEventPayload, Phase } from '../types';

type UnknownRecord = Record<string, unknown>;

const VALID_PHASES: Phase[] = [
  'idle',
  'thinking',
  'callingTool',
  'streaming',
  'interrupted',
  'done',
];

function asRecord(value: unknown): UnknownRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as UnknownRecord;
}

function pickString(record: UnknownRecord, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.length > 0) {
      return value;
    }
  }
  return null;
}

function pickOptionalString(record: UnknownRecord, ...keys: string[]): string | null | undefined {
  for (const key of keys) {
    if (!(key in record)) {
      continue;
    }
    const value = record[key];
    if (value == null) {
      return null;
    }
    if (typeof value === 'string') {
      return value;
    }
    return undefined;
  }
  return undefined;
}

function pickNumber(record: UnknownRecord, ...keys: string[]): number | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
  }
  return null;
}

function toPhase(value: unknown): Phase | null {
  if (typeof value !== 'string') {
    return null;
  }

  if ((VALID_PHASES as string[]).includes(value)) {
    return value as Phase;
  }

  return null;
}

function invalidEvent(reason: string, raw: unknown): AgentEventPayload {
  return {
    event: 'error',
    data: {
      code: 'invalid_agent_event',
      message: `${reason}: ${safeStringify(raw)}`,
      turnId: null,
    },
  };
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export function normalizeAgentEvent(raw: unknown): AgentEventPayload {
  const payload = asRecord(raw);
  if (!payload) {
    return invalidEvent('event payload is not an object', raw);
  }

  const event = pickString(payload, 'event');
  if (!event) {
    return invalidEvent('event field is missing', raw);
  }

  const data = asRecord(payload.data);
  if (!data) {
    return invalidEvent('data field is missing', raw);
  }

  if (event === 'sessionStarted') {
    const sessionId = pickString(data, 'sessionId', 'session_id') ?? 'unknown-session';
    return { event: 'sessionStarted', data: { sessionId } };
  }

  if (event === 'phaseChanged') {
    const phase = toPhase(data.phase);
    if (!phase) {
      return invalidEvent('phaseChanged.phase is invalid', raw);
    }
    return {
      event: 'phaseChanged',
      data: {
        phase,
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
      },
    };
  }

  if (event === 'modelDelta') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const delta = pickString(data, 'delta');
    if (!turnId || delta == null) {
      return invalidEvent('modelDelta requires turnId and delta', raw);
    }
    return { event: 'modelDelta', data: { turnId, delta } };
  }

  if (event === 'assistantMessage') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const content = pickString(data, 'content');
    if (!turnId || content == null) {
      return invalidEvent('assistantMessage requires turnId and content', raw);
    }
    return { event: 'assistantMessage', data: { turnId, content } };
  }

  if (event === 'toolCallStart') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const toolCallId = pickString(data, 'toolCallId', 'tool_call_id') ?? 'unknown';
    const toolName = pickString(data, 'toolName', 'tool_name') ?? '(unknown tool)';
    if (!turnId) {
      return invalidEvent('toolCallStart requires turnId', raw);
    }
    return {
      event: 'toolCallStart',
      data: {
        turnId,
        toolCallId,
        toolName,
        args: data.args ?? null,
      },
    };
  }

  if (event === 'toolCallResult') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const result = asRecord(data.result);
    if (!turnId || !result) {
      return invalidEvent('toolCallResult requires turnId and result', raw);
    }

    const toolCallId = pickString(result, 'toolCallId', 'tool_call_id') ?? 'unknown';
    const toolName = pickString(result, 'toolName', 'tool_name') ?? '';
    const output = pickString(result, 'output') ?? '';
    const durationMs = pickNumber(result, 'durationMs', 'duration_ms') ?? 0;
    const ok = result.ok === true;
    const error = pickOptionalString(result, 'error');

    return {
      event: 'toolCallResult',
      data: {
        turnId,
        result: {
          toolCallId,
          toolName,
          ok,
          output,
          error: error ?? undefined,
          metadata: result.metadata,
          durationMs,
        },
      },
    };
  }

  if (event === 'turnDone') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    if (!turnId) {
      return invalidEvent('turnDone requires turnId', raw);
    }
    return { event: 'turnDone', data: { turnId } };
  }

  if (event === 'error') {
    const code = pickString(data, 'code') ?? 'agent_error';
    const message = pickString(data, 'message') ?? 'unknown error';
    return {
      event: 'error',
      data: {
        code,
        message,
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
      },
    };
  }

  return invalidEvent(`unknown event type: ${event}`, raw);
}
