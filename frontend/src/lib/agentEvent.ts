//! # Agent 事件处理
//!
//! 将 SSE 接收的原始事件规范化为前端可用的格式。

import type {
  AgentEventPayload,
  CompactTrigger,
  InvocationKind,
  Phase,
  SubRunResult,
  SubRunStorageMode,
  ToolOutputStream,
} from '../types';
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
const VALID_INVOCATION_KINDS: InvocationKind[] = ['subRun', 'rootExecution'];
const VALID_SUBRUN_STORAGE_MODES: SubRunStorageMode[] = ['sharedSession', 'independentSession'];

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

function toCompactTrigger(value: unknown): CompactTrigger | null {
  if (value === 'auto' || value === 'manual') {
    return value;
  }
  return null;
}

function toInvocationKind(value: unknown): InvocationKind | null {
  if (typeof value !== 'string') {
    return null;
  }
  if ((VALID_INVOCATION_KINDS as string[]).includes(value)) {
    return value as InvocationKind;
  }
  return null;
}

function toSubRunStorageMode(value: unknown): SubRunStorageMode | null {
  if (typeof value !== 'string') {
    return null;
  }
  if ((VALID_SUBRUN_STORAGE_MODES as string[]).includes(value)) {
    return value as SubRunStorageMode;
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

function pickAgentContext(data: Record<string, unknown>) {
  const agentId = pickOptionalString(data, 'agentId', 'agent_id') ?? undefined;
  const parentTurnId = pickOptionalString(data, 'parentTurnId', 'parent_turn_id') ?? undefined;
  const agentProfile = pickOptionalString(data, 'agentProfile', 'agent_profile') ?? undefined;
  const subRunId = pickOptionalString(data, 'subRunId', 'sub_run_id') ?? undefined;
  const childSessionId =
    pickOptionalString(data, 'childSessionId', 'child_session_id') ?? undefined;
  const invocationKind = toInvocationKind(data.invocationKind ?? data.invocation_kind) ?? undefined;
  const storageMode = toSubRunStorageMode(data.storageMode ?? data.storage_mode) ?? undefined;
  return {
    ...(agentId ? { agentId } : {}),
    ...(parentTurnId ? { parentTurnId } : {}),
    ...(agentProfile ? { agentProfile } : {}),
    ...(subRunId ? { subRunId } : {}),
    ...(childSessionId ? { childSessionId } : {}),
    ...(invocationKind ? { invocationKind } : {}),
    ...(storageMode ? { storageMode } : {}),
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
    return { event: 'userMessage', data: { turnId, content, ...pickAgentContext(data) } };
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
        ...pickAgentContext(data),
      },
    };
  }

  if (event === 'modelDelta') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const delta = pickStringAllowEmpty(data, 'delta');
    if (!turnId || delta === undefined) {
      return invalidEvent('modelDelta requires turnId and delta', raw);
    }
    return { event: 'modelDelta', data: { turnId, delta, ...pickAgentContext(data) } };
  }

  if (event === 'thinkingDelta') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const delta = pickStringAllowEmpty(data, 'delta');
    if (!turnId || delta === undefined) {
      return invalidEvent('thinkingDelta requires turnId and delta', raw);
    }
    return { event: 'thinkingDelta', data: { turnId, delta, ...pickAgentContext(data) } };
  }

  if (event === 'assistantMessage') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    const content = pickStringAllowEmpty(data, 'content');
    const reasoningContent =
      pickOptionalString(data, 'reasoningContent', 'reasoning_content') ?? undefined;
    // Anthropic 工具回合经常会发送空 content 的 assistantMessage 作为阶段分隔；
    // 这属于协议内合法状态，不能再误报 invalid_agent_event。
    if (!turnId || content === undefined) {
      return invalidEvent('assistantMessage requires turnId and content', raw);
    }
    return {
      event: 'assistantMessage',
      data: {
        turnId,
        content,
        reasoningContent,
        ...pickAgentContext(data),
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
        ...pickAgentContext(data),
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
        ...pickAgentContext(data),
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
    const truncated = result.truncated === true;

    return {
      event: 'toolCallResult',
      data: {
        turnId,
        ...pickAgentContext(data),
        result: {
          toolCallId,
          toolName,
          ok,
          output,
          error: error ?? undefined,
          metadata: result.metadata,
          durationMs,
          truncated,
        },
      },
    };
  }

  if (event === 'promptMetrics') {
    const stepIndex = pickNumber(data, 'stepIndex', 'step_index');
    const estimatedTokens = pickNumber(data, 'estimatedTokens', 'estimated_tokens');
    const contextWindow = pickNumber(data, 'contextWindow', 'context_window');
    const effectiveWindow = pickNumber(data, 'effectiveWindow', 'effective_window');
    const thresholdTokens = pickNumber(data, 'thresholdTokens', 'threshold_tokens');
    const truncatedToolResults = pickNumber(data, 'truncatedToolResults', 'truncated_tool_results');

    if (
      stepIndex === null ||
      estimatedTokens === null ||
      contextWindow === null ||
      effectiveWindow === null ||
      thresholdTokens === null ||
      truncatedToolResults === null
    ) {
      return invalidEvent('promptMetrics requires the full snapshot fields', raw);
    }

    return {
      event: 'promptMetrics',
      data: {
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
        stepIndex,
        estimatedTokens,
        contextWindow,
        effectiveWindow,
        thresholdTokens,
        truncatedToolResults,
        providerInputTokens:
          pickNumber(data, 'providerInputTokens', 'provider_input_tokens') ?? undefined,
        providerOutputTokens:
          pickNumber(data, 'providerOutputTokens', 'provider_output_tokens') ?? undefined,
        cacheCreationInputTokens:
          pickNumber(data, 'cacheCreationInputTokens', 'cache_creation_input_tokens') ?? undefined,
        cacheReadInputTokens:
          pickNumber(data, 'cacheReadInputTokens', 'cache_read_input_tokens') ?? undefined,
        ...pickAgentContext(data),
      },
    };
  }

  if (event === 'compactApplied') {
    const trigger = toCompactTrigger(data.trigger);
    const summary = pickStringAllowEmpty(data, 'summary');
    const preservedRecentTurns = pickNumber(data, 'preservedRecentTurns', 'preserved_recent_turns');
    if (!trigger || summary === undefined || preservedRecentTurns === null) {
      return invalidEvent('compactApplied requires trigger, summary and preservedRecentTurns', raw);
    }
    return {
      event: 'compactApplied',
      data: {
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
        trigger,
        summary,
        preservedRecentTurns,
        ...pickAgentContext(data),
      },
    };
  }

  if (event === 'subRunStarted') {
    const storageMode = toSubRunStorageMode(
      data.resolvedOverrides && typeof data.resolvedOverrides === 'object'
        ? ((data.resolvedOverrides as Record<string, unknown>).storageMode ??
            (data.resolvedOverrides as Record<string, unknown>).storage_mode)
        : undefined
    );
    const resolvedOverrides = asRecord(data.resolvedOverrides);
    const resolvedLimits = asRecord(data.resolvedLimits);
    if (!resolvedOverrides || !resolvedLimits || !storageMode) {
      return invalidEvent('subRunStarted requires resolvedOverrides and resolvedLimits', raw);
    }
    return {
      event: 'subRunStarted',
      data: {
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
        ...pickAgentContext(data),
        resolvedOverrides: {
          storageMode,
          inheritSystemInstructions: resolvedOverrides.inheritSystemInstructions === true,
          inheritProjectInstructions: resolvedOverrides.inheritProjectInstructions === true,
          inheritWorkingDir: resolvedOverrides.inheritWorkingDir === true,
          inheritPolicyUpperBound: resolvedOverrides.inheritPolicyUpperBound === true,
          inheritCancelToken: resolvedOverrides.inheritCancelToken === true,
          includeCompactSummary: resolvedOverrides.includeCompactSummary === true,
          includeRecentTail: resolvedOverrides.includeRecentTail === true,
          includeRecoveryRefs: resolvedOverrides.includeRecoveryRefs === true,
          includeParentFindings: resolvedOverrides.includeParentFindings === true,
        },
        resolvedLimits: {
          maxSteps: pickNumber(resolvedLimits, 'maxSteps', 'max_steps') ?? undefined,
          tokenBudget: pickNumber(resolvedLimits, 'tokenBudget', 'token_budget') ?? undefined,
          allowedTools: Array.isArray(resolvedLimits.allowedTools)
            ? resolvedLimits.allowedTools.filter(
                (value): value is string => typeof value === 'string'
              )
            : [],
        },
      },
    };
  }

  if (event === 'subRunFinished') {
    const result = asRecord(data.result);
    if (!result) {
      return invalidEvent('subRunFinished requires result', raw);
    }
    const status = result.status;
    if (
      status !== 'running' &&
      status !== 'completed' &&
      status !== 'failed' &&
      status !== 'aborted' &&
      status !== 'token_exceeded'
    ) {
      return invalidEvent('subRunFinished.result.status is invalid', raw);
    }
    const handoff = asRecord(result.handoff);
    const failure = asRecord(result.failure);

    return {
      event: 'subRunFinished',
      data: {
        turnId: pickOptionalString(data, 'turnId', 'turn_id') ?? null,
        ...pickAgentContext(data),
        result: {
          status,
          handoff: handoff
            ? {
                summary: pickString(handoff, 'summary') ?? '',
                artifacts: Array.isArray(handoff.artifacts)
                  ? handoff.artifacts
                      .map((value) => asRecord(value))
                      .filter((value): value is Record<string, unknown> => Boolean(value))
                      .map((artifact) => ({
                        kind: pickString(artifact, 'kind') ?? 'unknown',
                        id: pickString(artifact, 'id') ?? 'unknown',
                        label: pickString(artifact, 'label') ?? 'artifact',
                        sessionId:
                          pickOptionalString(artifact, 'sessionId', 'session_id') ?? undefined,
                        storageSeq: pickNumber(artifact, 'storageSeq', 'storage_seq') ?? undefined,
                        uri: pickOptionalString(artifact, 'uri') ?? undefined,
                      }))
                  : [],
                findings: Array.isArray(handoff.findings)
                  ? handoff.findings.filter((value): value is string => typeof value === 'string')
                  : [],
              }
            : undefined,
          failure: failure
            ? {
                code:
                  pickString(failure, 'code') === 'transport' ||
                  pickString(failure, 'code') === 'provider_http' ||
                  pickString(failure, 'code') === 'stream_parse' ||
                  pickString(failure, 'code') === 'interrupted' ||
                  pickString(failure, 'code') === 'internal'
                    ? (pickString(failure, 'code') as NonNullable<SubRunResult['failure']>['code'])
                    : 'internal',
                displayMessage: pickString(failure, 'displayMessage', 'display_message') ?? '',
                technicalMessage:
                  pickString(failure, 'technicalMessage', 'technical_message') ?? '',
                retryable: failure.retryable === true,
              }
            : undefined,
        },
        stepCount: pickNumber(data, 'stepCount', 'step_count') ?? 0,
        estimatedTokens: pickNumber(data, 'estimatedTokens', 'estimated_tokens') ?? 0,
      },
    };
  }

  if (event === 'turnDone') {
    const turnId = pickString(data, 'turnId', 'turn_id');
    if (!turnId) {
      return invalidEvent('turnDone requires turnId', raw);
    }
    return { event: 'turnDone', data: { turnId, ...pickAgentContext(data) } };
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
        ...pickAgentContext(data),
      },
    };
  }

  return invalidEvent(`unknown event type: ${event}`, raw);
}
