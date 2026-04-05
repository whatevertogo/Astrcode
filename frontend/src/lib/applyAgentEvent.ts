import type { Action, AgentEventPayload, Phase } from '../types';
import { uuid } from '../utils/uuid';
import { releaseTurnMapping, resolveSessionForTurn } from './turnRouting';

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
  const scheduleMicrotask = context.scheduleMicrotask ?? ((callback: () => void) => callback());
  const resolveSessionId = (turnId?: string | null): string | null => {
    return resolveSessionForTurn(
      context.turnSessionMapRef.current,
      context.pendingSubmitSessionRef.current,
      turnId,
      context.activeSessionIdRef.current
    );
  };

  switch (event.event) {
    case 'sessionStarted':
      break;

    case 'userMessage': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      context.dispatch({
        type: 'UPSERT_USER_MESSAGE',
        sessionId,
        turnId: event.data.turnId,
        content: event.data.content,
      });
      break;
    }

    case 'phaseChanged': {
      if (event.data.turnId) {
        resolveSessionId(event.data.turnId);
      }
      context.phaseRef.current = event.data.phase;
      context.dispatch({ type: 'SET_PHASE', phase: event.data.phase });
      break;
    }

    case 'modelDelta': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      context.dispatch({
        type: 'APPEND_DELTA',
        sessionId,
        turnId: event.data.turnId,
        delta: event.data.delta,
      });
      break;
    }

    case 'thinkingDelta': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      context.dispatch({
        type: 'APPEND_REASONING_DELTA',
        sessionId,
        turnId: event.data.turnId,
        delta: event.data.delta,
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
      context.dispatch({
        type: 'FINALIZE_ASSISTANT',
        sessionId,
        turnId: event.data.turnId,
        content: event.data.content,
        reasoningText: event.data.reasoningContent,
      });
      break;
    }

    case 'toolCallStart': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      context.dispatch({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'toolCall',
          turnId: event.data.turnId,
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
      context.dispatch({
        type: 'APPEND_TOOL_CALL_DELTA',
        sessionId,
        turnId: event.data.turnId,
        toolCallId: event.data.toolCallId,
        toolName: event.data.toolName,
        stream: event.data.stream,
        delta: event.data.delta,
      });
      break;
    }

    case 'toolCallResult': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (!sessionId) {
        break;
      }
      context.dispatch({
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
      });
      break;
    }

    case 'compactApplied': {
      const sessionId = event.data.turnId
        ? resolveSessionId(event.data.turnId)
        : context.activeSessionIdRef.current;
      if (!sessionId) {
        break;
      }
      context.dispatch({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'compact',
          turnId: event.data.turnId ?? null,
          trigger: event.data.trigger,
          summary: event.data.summary,
          preservedRecentTurns: event.data.preservedRecentTurns,
          timestamp: Date.now(),
        },
      });
      break;
    }

    case 'turnDone': {
      const sessionId = resolveSessionId(event.data.turnId);
      if (sessionId) {
        context.dispatch({ type: 'END_STREAMING', sessionId, turnId: event.data.turnId });
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
      if (sessionId && event.data.code !== 'interrupted') {
        context.dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
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
      context.dispatch({ type: 'SET_PHASE', phase: 'idle' });
      break;
    }
  }
}
