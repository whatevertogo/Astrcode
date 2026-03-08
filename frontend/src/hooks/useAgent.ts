import { useEffect, useRef } from 'react';
import type { Dispatch } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { Action, AppState, Phase, ToolStatus } from '../types';
import { uuid } from '../utils/uuid';

// Raw payload shape from Rust's AgentEvent (serde flatten + tag/content)
interface AgentEventRaw {
  protocolVersion: number;
  event: string;
  data: Record<string, unknown>;
}

export function useAgent(
  state: AppState,
  dispatch: Dispatch<Action>,
) {
  const activeSessionIdRef = useRef<string | null>(state.activeSessionId);
  const dispatchRef = useRef(dispatch);
  const defaultProjectIdRef = useRef(state.projects[0]?.id ?? null);

  // Keep refs in sync
  useEffect(() => {
    activeSessionIdRef.current = state.activeSessionId;
  }, [state.activeSessionId]);

  useEffect(() => {
    dispatchRef.current = dispatch;
  }, [dispatch]);

  // Fetch working dir on mount and set it on the default project
  useEffect(() => {
    const projectId = defaultProjectIdRef.current;
    if (!projectId) return;
    invoke<string>('get_working_dir')
      .then((dir) => {
        dispatchRef.current({
          type: 'SET_WORKING_DIR',
          projectId,
          workingDir: dir,
        });
      })
      .catch(console.error);
  }, []);

  // Subscribe to backend events
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const handleAgentEvent = (event: { payload: AgentEventRaw }) => {
      const payload = event.payload;
      const sid = activeSessionIdRef.current;
      const d = dispatchRef.current;

      switch (payload.event) {
        case 'phaseChanged':
          d({ type: 'SET_PHASE', phase: payload.data.phase as Phase });
          break;

        case 'modelDelta':
          if (!sid) break;
          d({
            type: 'APPEND_DELTA',
            sessionId: sid,
            delta: payload.data.delta as string,
          });
          break;

        case 'turnDone':
          if (!sid) break;
          d({ type: 'END_STREAMING', sessionId: sid });
          break;

        case 'toolCallStart':
          if (!sid) break;
          d({
            type: 'ADD_MESSAGE',
            sessionId: sid,
            message: {
              id: uuid(),
              kind: 'toolCall',
              toolCallId: payload.data.toolCallId as string,
              toolName: payload.data.toolName as string,
              status: 'running' as ToolStatus,
              args: payload.data.args,
              timestamp: Date.now(),
            },
          });
          break;

        case 'toolCallResult': {
          if (!sid) break;
          const result = payload.data.result as {
            toolCallId: string;
            ok: boolean;
            output: string;
            error?: string;
            durationMs: number;
          };
          d({
            type: 'UPDATE_TOOL_CALL',
            sessionId: sid,
            toolCallId: result.toolCallId,
            status: (result.ok ? 'ok' : 'fail') as ToolStatus,
            output: result.output,
            error: result.error,
            durationMs: result.durationMs,
          });
          break;
        }

        case 'error':
          if (!sid) break;
          d({
            type: 'ADD_MESSAGE',
            sessionId: sid,
            message: {
              id: uuid(),
              kind: 'assistant',
              text: `错误 [${payload.data.code}]: ${payload.data.message}`,
              streaming: false,
              timestamp: Date.now(),
            },
          });
          break;
      }
    };

    void listen<AgentEventRaw>('agent-event', handleAgentEvent)
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((error) => {
        if (!disposed) {
          console.error(error);
        }
      });

    return () => {
      disposed = true;
      unlisten?.();
      unlisten = null;
    };
  }, []);

  const submitPrompt = async (text: string) => {
    const sid = activeSessionIdRef.current;
    if (!sid) return;
    dispatchRef.current({
      type: 'ADD_MESSAGE',
      sessionId: sid,
      message: {
        id: uuid(),
        kind: 'user',
        text,
        timestamp: Date.now(),
      },
    });
    try {
      await invoke('submit_prompt', { text });
    } catch (err) {
      console.error('submit_prompt error:', err);
    }
  };

  const interrupt = async () => {
    try {
      await invoke('interrupt');
    } catch (err) {
      console.error('interrupt error:', err);
    }
  };

  return { submitPrompt, interrupt };
}
