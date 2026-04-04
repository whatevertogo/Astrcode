import { useCallback, type Dispatch, type MutableRefObject } from 'react';
import type { AgentEventPayload, Action, Phase } from '../types';
import { uuid } from '../utils/uuid';
import { releaseTurnMapping, resolveSessionForTurn } from '../lib/turnRouting';

interface AgentEventHandlerOptions {
  activeSessionIdRef: MutableRefObject<string | null>;
  pendingSubmitSessionRef: MutableRefObject<string[]>;
  turnSessionMapRef: MutableRefObject<Record<string, string>>;
  phaseRef: MutableRefObject<Phase>;
  dispatch: Dispatch<Action>;
}

export function useAgentEventHandler({
  activeSessionIdRef,
  pendingSubmitSessionRef,
  turnSessionMapRef,
  phaseRef,
  dispatch,
}: AgentEventHandlerOptions) {
  return useCallback(
    (event: AgentEventPayload) => {
      // Centralizing turn routing keeps branch-switch handling consistent across every event type.
      const resolveSessionId = (turnId?: string | null): string | null => {
        return resolveSessionForTurn(
          turnSessionMapRef.current,
          pendingSubmitSessionRef.current,
          turnId,
          activeSessionIdRef.current
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
          dispatch({
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
          dispatch({ type: 'SET_PHASE', phase: event.data.phase });
          break;
        }

        case 'modelDelta': {
          const sessionId = resolveSessionId(event.data.turnId);
          if (!sessionId) {
            break;
          }
          // 流式文本更新需要高优先级，不能使用 startTransition
          // 否则会被 React 批量处理导致"一坨一坨"输出
          dispatch({
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
          // 流式 thinking 更新同样需要高优先级
          dispatch({
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
          // Anthropic 工具回合会产出空 assistantMessage 作为中间状态，
          // 这里直接忽略，避免在 UI 中生成空白气泡。
          if (!event.data.content && !event.data.reasoningContent) {
            break;
          }
          dispatch({
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
          dispatch({
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
          // 工具调用流式输出也需要高优先级
          dispatch({
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
          dispatch({
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

        case 'turnDone': {
          const sessionId = resolveSessionId(event.data.turnId);
          if (sessionId) {
            dispatch({ type: 'END_STREAMING', sessionId, turnId: event.data.turnId });
          }
          releaseTurnMapping(turnSessionMapRef.current, event.data.turnId);
          queueMicrotask(() => {
            if (phaseRef.current !== 'idle') {
              dispatch({ type: 'SET_PHASE', phase: 'idle' });
            }
          });
          break;
        }

        case 'error': {
          const sessionId = resolveSessionId(event.data.turnId ?? null);
          if (sessionId && event.data.code !== 'interrupted') {
            dispatch({
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
            releaseTurnMapping(turnSessionMapRef.current, event.data.turnId);
          }
          dispatch({ type: 'SET_PHASE', phase: 'idle' });
          break;
        }
      }
    },
    [activeSessionIdRef, dispatch, pendingSubmitSessionRef, phaseRef, turnSessionMapRef]
  );
}
