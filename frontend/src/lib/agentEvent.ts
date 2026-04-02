//! # Agent 事件处理
//!
//! 将 SSE 接收的原始事件规范化为前端可用的格式。

import type { AgentEventPayload, Phase, ToolOutputStream } from '../types';
import {
  asRecord,
  pickString,
  pickStringAllowEmpty,
  pickOptionalString,
  pickNumber,
  safeStringify,
} from './shared';

/// 支持的协议版本
const SUPPORTED_PROTOCOL_VERSION = 1;

const VALID_PHASES: Phase[] = [
  'idle',
  'thinking',
  'callingTool',
  'streaming',
  'interrupted',
  'done',
];
const VALID_TOOL_OUTPUT_STREAMS: ToolOutputStream[] = ['stdout', 'stderr'];

function toPhase(value: unknown): Phase | null {
  if (typeof value !== 'string') {
    return null;
  }

  if ((VALID_PHASES as string[]).includes(value)) {
    return value as Phase;
  }

  return null;
}

function toToolOutputStream(value: unknown): ToolOutputStream | null {
  if (typeof value !== 'string') {
    return null;
  }

  if ((VALID_TOOL_OUTPUT_STREAMS as string[]).includes(value)) {
    return value as ToolOutputStream;
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

export function normalizeAgentEvent(raw: unknown): AgentEventPayload {
  const payload = asRecord(raw);
  if (!payload) {
    return invalidEvent('event payload is not an object', raw);
  }

  const protocolVersion = pickNumber(payload, 'protocolVersion', 'protocol_version');
  if (protocolVersion === null) {
    return invalidEvent('protocolVersion field is missing', raw);
  }
  if (protocolVersion !== SUPPORTED_PROTOCOL_VERSION) {
    return invalidEvent(
      `unsupported protocolVersion ${protocolVersion} (expected ${SUPPORTED_PROTOCOL_VERSION})`,
      raw
    );
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

  if (event === 'userMessage') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const content = pickStringAllowEmpty(data, 'content');
    if (!turnId || content === undefined) {
      return invalidEvent('userMessage requires turnId and content', raw);
    }
    return { event: 'userMessage', data: { turnId, content } };
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
    const delta = pickStringAllowEmpty(data, 'delta');
    if (!turnId || delta === undefined) {
      return invalidEvent('modelDelta requires turnId and delta', raw);
    }
    return { event: 'modelDelta', data: { turnId, delta } };
  }

  if (event === 'thinkingDelta') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const delta = pickStringAllowEmpty(data, 'delta');
    if (!turnId || delta === undefined) {
      return invalidEvent('thinkingDelta requires turnId and delta', raw);
    }
    return { event: 'thinkingDelta', data: { turnId, delta } };
  }

  if (event === 'assistantMessage') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const content = pickStringAllowEmpty(data, 'content');
    const reasoningContent =
      pickOptionalString(data, 'reasoningContent', 'reasoning_content') ?? undefined;
    // assistantMessage 可能只携带 reasoning；这时 content 允许为空字符串，
    // 否则前端会把合法的中间态消息误报为协议错误。
    if (!turnId || content === undefined || (content.length === 0 && !reasoningContent?.length)) {
      return invalidEvent('assistantMessage requires turnId and content', raw);
    }
    return {
      event: 'assistantMessage',
      data: {
        turnId,
        content,
        reasoningContent,
      },
    };
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

  if (event === 'toolCallDelta') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const toolCallId = pickString(data, 'toolCallId', 'tool_call_id') ?? 'unknown';
    const toolName = pickString(data, 'toolName', 'tool_name') ?? '(unknown tool)';
    const stream = toToolOutputStream(data.stream);
    const delta = pickStringAllowEmpty(data, 'delta');
    if (!turnId || !stream || delta === undefined) {
      return invalidEvent('toolCallDelta requires turnId, stream and delta', raw);
    }
    return {
      event: 'toolCallDelta',
      data: {
        turnId,
        toolCallId,
        toolName,
        stream,
        delta,
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
