import type { Action, AgentEventPayload, AtomicAction, Phase } from '../types';
import { uuid } from '../utils/uuid';
import { releaseTurnMapping, resolveSessionForTurn } from './turnRouting';
import { logger } from './logger';

interface MutableValue<T> {
  current: T;
}

export interface AgentEventDispatchContext {
  activeSessionIdRef: MutableValue<string | null>;
  pendingSubmitSessionRef: MutableValue<string[]>;
  turnSessionMapRef: MutableValue<Record<string, string>>;
  phaseRef: MutableValue<Phase>;
  dispatch: (action: Action) => void;
  scheduleMicrotask?: (callback: () => void) => void;
}

/// 将单条 Agent 事件应用到前端状态。
///
/// 这里把“事件 → action”的翻译规则收敛到一个纯函数入口，原因是：
/// 1. 历史回放和实时 SSE 必须共享同一套协议语义，避免再次分叉；
/// 2. turn 路由、phase 兜底、工具卡片补全这些细节已经很容易漂移，不适合复制。
export function applyAgentEvent(
  context: AgentEventDispatchContext,
  event: AgentEventPayload
): void {
  applyAgentEvents(context, [event]);
}

export function applyAgentEvents(
  context: AgentEventDispatchContext,
  events: AgentEventPayload[]
): void {
  const actions: AtomicAction[] = [];
  for (const event of events) {
    collectAgentEventActions(context, event, (action) => {
      actions.push(action);
    });
  }

  const mergedActions = mergeStreamingActions(actions);
  if (mergedActions.length === 0) {
    return;
  }
  if (mergedActions.length === 1) {
    context.dispatch(mergedActions[0]);
    return;
  }
  context.dispatch({ type: 'APPLY_AGENT_EVENTS_BATCH', actions: mergedActions });
}

function collectAgentEventActions(
  context: AgentEventDispatchContext,
  event: AgentEventPayload,
  emit: (action: AtomicAction) => void
): void {
  const scheduleMicrotask = context.scheduleMicrotask ?? ((callback: () => void) => callback());
  const resolveSessionId = (turnId?: string | null): string | null => {
    return resolveSessionForTurn(
      context.turnSessionMapRef.current,
      context.pendingSubmitSessionRef.current,
      turnId,
      context.activeSessionIdRef.current
    );
  };
  const agentFields =
    'agentId' in event.data ||
    'parentTurnId' in event.data ||
    'agentProfile' in event.data ||
    'subRunId' in event.data
      ? {
          agentId: 'agentId' in event.data ? event.data.agentId : undefined,
          parentTurnId: 'parentTurnId' in event.data ? event.data.parentTurnId : undefined,
          parentSubRunId: 'parentSubRunId' in event.data ? event.data.parentSubRunId : undefined,
          agentProfile: 'agentProfile' in event.data ? event.data.agentProfile : undefined,
          subRunId: 'subRunId' in event.data ? event.data.subRunId : undefined,
          executionId: 'executionId' in event.data ? event.data.executionId : undefined,
          invocationKind: 'invocationKind' in event.data ? event.data.invocationKind : undefined,
          storageMode: 'storageMode' in event.data ? event.data.storageMode : undefined,
          childSessionId: 'childSessionId' in event.data ? event.data.childSessionId : undefined,
        }
      : {
          agentId: undefined,
          parentTurnId: undefined,
          parentSubRunId: undefined,
          agentProfile: undefined,
          subRunId: undefined,
          executionId: undefined,
          invocationKind: undefined,
          storageMode: undefined,
          childSessionId: undefined,
        };

  switch (event.event) {
    case 'sessionStarted':
      break;

    case 'userMessage': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'UPSERT_USER_MESSAGE',
        sessionId,
        turnId: event.data.turnId,
        content: event.data.content,
        ...agentFields,
      });
      break;
    }

    case 'phaseChanged': {
      if (event.data.turnId) {
        resolveSessionId(event.data.turnId);
      }
      context.phaseRef.current = event.data.phase;
      emit({ type: 'SET_PHASE', phase: event.data.phase });
      break;
    }

    case 'modelDelta': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'APPEND_DELTA',
        sessionId,
        turnId: event.data.turnId,
        delta: event.data.delta,
        ...agentFields,
      });
      break;
    }

    case 'thinkingDelta': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'APPEND_REASONING_DELTA',
        sessionId,
        turnId: event.data.turnId,
        delta: event.data.delta,
        ...agentFields,
      });
      break;
    }

    case 'assistantMessage': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      if (!event.data.content && !event.data.reasoningContent) {
        break;
      }
      emit({
        type: 'FINALIZE_ASSISTANT',
        sessionId,
        turnId: event.data.turnId,
        content: event.data.content,
        reasoningText: event.data.reasoningContent,
        ...agentFields,
      });
      break;
    }

    case 'toolCallStart': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'toolCall',
          turnId: event.data.turnId,
          ...agentFields,
          toolCallId: event.data.toolCallId,
          toolName: event.data.toolName,
          status: 'running',
          args: event.data.args,
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'toolCallDelta': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'APPEND_TOOL_CALL_DELTA',
        sessionId,
        turnId: event.data.turnId,
        toolCallId: event.data.toolCallId,
        toolName: event.data.toolName,
        stream: event.data.stream,
        delta: event.data.delta,
        ...agentFields,
      });
      break;
    }

    case 'toolCallResult': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      emit({
        type: 'UPDATE_TOOL_CALL',
        sessionId,
        turnId: event.data.turnId,
        toolCallId: event.data.result.toolCallId,
        toolName: event.data.result.toolName,
        status: event.data.result.ok ? 'ok' : 'fail',
        output: event.data.result.output,
        error: event.data.result.error,
        metadata: event.data.result.metadata,
        durationMs: event.data.result.durationMs,
        truncated: event.data.result.truncated,
        ...agentFields,
      });
      break;
    }

    case 'promptMetrics': {
      const sessionId = event.data.turnId
        ? resolveSessionId(event.data.turnId)
        : context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      emit({
        type: 'UPSERT_PROMPT_METRICS',
        sessionId,
        turnId: event.data.turnId ?? null,
        ...agentFields,
        stepIndex: event.data.stepIndex,
        estimatedTokens: event.data.estimatedTokens,
        contextWindow: event.data.contextWindow,
        effectiveWindow: event.data.effectiveWindow,
        thresholdTokens: event.data.thresholdTokens,
        truncatedToolResults: event.data.truncatedToolResults,
        providerInputTokens: event.data.providerInputTokens,
        providerOutputTokens: event.data.providerOutputTokens,
        cacheCreationInputTokens: event.data.cacheCreationInputTokens,
        cacheReadInputTokens: event.data.cacheReadInputTokens,
        providerCacheMetricsSupported: event.data.providerCacheMetricsSupported,
        promptCacheReuseHits: event.data.promptCacheReuseHits,
        promptCacheReuseMisses: event.data.promptCacheReuseMisses,
      });
      break;
    }

    case 'agentMailboxQueued':
    case 'agentMailboxBatchStarted':
    case 'agentMailboxBatchAcked':
    case 'agentMailboxDiscarded':
      // Why: mailbox 事件目前只用于协议完整性与调试观测；
      // UI 主投影仍以 sub-run / child-session 事件为准，避免重复噪声。
      break;

    case 'compactApplied': {
      const sessionId = event.data.turnId
        ? resolveSessionId(event.data.turnId)
        : context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      emit({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'compact',
          turnId: event.data.turnId ?? null,
          ...agentFields,
          trigger: event.data.trigger,
          summary: event.data.summary,
          preservedRecentTurns: event.data.preservedRecentTurns,
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'subRunStarted': {
      const sessionId = context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      const toolCallId = event.data.toolCallId;
      emit({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'subRunStart',
          turnId: event.data.turnId ?? null,
          ...agentFields,
          ...(toolCallId ? { toolCallId } : {}),
          resolvedOverrides: event.data.resolvedOverrides,
          resolvedLimits: event.data.resolvedLimits,
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'subRunFinished': {
      const sessionId = context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      const toolCallId = event.data.toolCallId;
      if (event.data.result.status === 'failed' && event.data.result.failure) {
        logger.modelError('applyAgentEvent', 'subRun failed', {
          code: event.data.result.failure.code,
          turnId: event.data.turnId,
          technicalMessage: event.data.result.failure.technicalMessage,
        });
      }
      emit({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'subRunFinish',
          turnId: event.data.turnId ?? null,
          ...agentFields,
          ...(toolCallId ? { toolCallId } : {}),
          result: event.data.result,
          stepCount: event.data.stepCount,
          estimatedTokens: event.data.estimatedTokens,
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'childSessionNotification': {
      const sessionId = context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      const openSessionId = event.data.childRef.openSessionId;
      emit({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'childSessionNotification',
          turnId: event.data.turnId ?? null,
          ...agentFields,
          childSessionId: openSessionId,
          childRef: event.data.childRef,
          notificationKind: event.data.kind,
          status: event.data.status,
          summary: event.data.summary,
          ...(event.data.sourceToolCallId ? { sourceToolCallId: event.data.sourceToolCallId } : {}),
          ...(event.data.finalReplyExcerpt
            ? { finalReplyExcerpt: event.data.finalReplyExcerpt }
            : {}),
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'turnDone': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (sessionId) {
        emit({ type: 'END_STREAMING', sessionId, turnId: event.data.turnId });
      }
      releaseTurnMapping(context.turnSessionMapRef.current, event.data.turnId);
      scheduleMicrotask(() => {
        if (context.phaseRef.current !== 'idle') {
          context.phaseRef.current = 'idle';
          context.dispatch({ type: 'SET_PHASE', phase: 'idle' });
        }
      });
      break;
    }

    case 'error': {
      const sessionId = resolveSessionId(event.data.turnId ?? null);
      if (event.data.code !== 'interrupted') {
        logger.modelError('applyAgentEvent', 'agent error event', {
          code: event.data.code,
          turnId: event.data.turnId,
          message: event.data.message,
        });
      }
      if (sessionId && event.data.code !== 'interrupted') {
        emit({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
            ...agentFields,
            text: `错误：${event.data.message}`,
            reasoningText: '',
            streaming: false,
            timestamp: Date.now(),
          },
        });
      }
      if (event.data.turnId) {
        releaseTurnMapping(context.turnSessionMapRef.current, event.data.turnId);
      }
      context.phaseRef.current = 'idle';
      emit({ type: 'SET_PHASE', phase: 'idle' });
      break;
    }
  }
}

function mergeStreamingActions(actions: AtomicAction[]): AtomicAction[] {
  const merged: AtomicAction[] = [];

  for (const action of actions) {
    const previous = merged[merged.length - 1];
    if (
      previous?.type === 'APPEND_DELTA' &&
      action.type === 'APPEND_DELTA' &&
      hasSameDeltaTarget(previous, action)
    ) {
      previous.delta += action.delta;
      continue;
    }
    if (
      previous?.type === 'APPEND_REASONING_DELTA' &&
      action.type === 'APPEND_REASONING_DELTA' &&
      hasSameDeltaTarget(previous, action)
    ) {
      previous.delta += action.delta;
      continue;
    }
    merged.push(action);
  }

  return merged;
}

function hasSameDeltaTarget(
  previous:
    | Extract<AtomicAction, { type: 'APPEND_DELTA' }>
    | Extract<AtomicAction, { type: 'APPEND_REASONING_DELTA' }>,
  next:
    | Extract<AtomicAction, { type: 'APPEND_DELTA' }>
    | Extract<AtomicAction, { type: 'APPEND_REASONING_DELTA' }>
): boolean {
  return (
    previous.sessionId === next.sessionId &&
    previous.turnId === next.turnId &&
    previous.subRunId === next.subRunId &&
    previous.executionId === next.executionId &&
    previous.agentId === next.agentId
  );
}
