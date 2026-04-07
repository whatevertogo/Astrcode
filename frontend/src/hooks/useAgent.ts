//! # Agent Hook
//!
//! Orchestrates API calls and SSE event streaming.
//!
//! ## Refactoring Notes
//!
//! API calls have been extracted into `lib/api/` so this hook only coordinates
//! state, lifecycle, and reconnection logic. Tests can now target individual API
//! modules in isolation.

import { useCallback, useEffect, useRef } from 'react';
import { normalizeAgentEvent } from '../lib/agentEvent';
import { getHostBridge } from '../lib/hostBridge';
import { consumeSseStream } from '../lib/sse/consumer';
import { buildSessionEventQueryString } from '../lib/sessionView';
import { ensureServerSession } from '../lib/serverAuth';
import { request } from '../lib/api/client';
import { listComposerOptions } from '../lib/api/composer';
import {
  compactSession,
  cancelSubRun,
  createSession,
  deleteProject,
  deleteSession,
  interruptSession,
  listSessionsWithMeta,
  loadSession,
  submitPrompt,
} from '../lib/api/sessions';
import { getConfig, saveActiveSelection } from '../lib/api/config';
import { getCurrentModel, listAvailableModels, testConnection } from '../lib/api/models';
import type {
  AgentEventPayload,
  ComposerOption,
  ConfigView,
  CurrentModelInfo,
  DeleteProjectResult,
  ModelOption,
  Phase,
  SessionMeta,
  TestResult,
} from '../types';
import type { SessionEventFilterQuery } from '../lib/sessionView';

export interface SessionSnapshot {
  events: AgentEventPayload[];
  cursor: string | null;
  phase: Phase;
}

export interface PromptSubmission {
  turnId: string;
  sessionId: string;
  branchedFromSessionId?: string;
}

// SSE 重连配置
const SSE_RECONNECT_BASE_DELAY_MS = 500;
const SSE_RECONNECT_MAX_DELAY_MS = 5_000;
const SSE_RECONNECT_FATAL_ATTEMPTS = 3;

/// 分发流错误事件
function dispatchStreamError(
  onEvent: (event: AgentEventPayload) => void,
  message: string,
  turnId: string | null = null
): void {
  onEvent({
    event: 'error',
    data: {
      code: 'event_stream_error',
      message,
      turnId,
    },
  });
}

function shouldRetryEventStream(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();
  return !message.includes('unauthorized') && !message.includes('403');
}

export function useAgent(onEvent: (event: AgentEventPayload) => void) {
  const onEventRef = useRef(onEvent);
  const streamAbortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const reconnectAttemptRef = useRef(0);
  const connectedSessionIdRef = useRef<string | null>(null);
  const connectedSessionFilterRef = useRef<SessionEventFilterQuery | undefined>(undefined);
  const lastEventIdRef = useRef<string | null>(null);

  // Generation counter to prevent race conditions when switching sessions
  const streamGenerationRef = useRef(0);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const failActiveConnection = useCallback(
    (message: string, turnId: string | null = null) => {
      // 这里显式结束本地流状态，而不是无限重试。
      // 原因是：当用户手动关闭 server 后，继续保持 busy 只会把 UI 卡死在“可中断但无法中断”的假状态。
      clearReconnectTimer();
      streamAbortRef.current?.abort();
      streamAbortRef.current = null;
      connectedSessionIdRef.current = null;
      connectedSessionFilterRef.current = undefined;
      lastEventIdRef.current = null;
      reconnectAttemptRef.current = 0;
      streamGenerationRef.current += 1;
      dispatchStreamError(onEventRef.current, message, turnId);
    },
    [clearReconnectTimer]
  );

  useEffect(() => {
    return () => {
      streamAbortRef.current?.abort();
      streamAbortRef.current = null;
      clearReconnectTimer();
    };
  }, [clearReconnectTimer]);

  const dispatchIncomingEvent = useCallback((rawEvent: unknown) => {
    onEventRef.current(normalizeAgentEvent(rawEvent));
  }, []);

  const connectSession = useCallback(
    async (
      sessionId: string,
      afterEventId?: string | null,
      filter?: SessionEventFilterQuery
    ): Promise<void> => {
      await ensureServerSession();
      clearReconnectTimer();
      streamAbortRef.current?.abort();

      // Increment generation to invalidate any pending operations from previous connections
      const generation = ++streamGenerationRef.current;

      connectedSessionIdRef.current = sessionId;
      connectedSessionFilterRef.current = filter;
      lastEventIdRef.current = afterEventId ?? null;
      reconnectAttemptRef.current = 0;

      const scheduleReconnect = (failureMessage: string) => {
        // Check if this connection is still active
        if (streamGenerationRef.current !== generation) {
          return;
        }
        if (connectedSessionIdRef.current !== sessionId) {
          return;
        }
        clearReconnectTimer();
        const attempt = reconnectAttemptRef.current + 1;
        reconnectAttemptRef.current = attempt;
        if (attempt >= SSE_RECONNECT_FATAL_ATTEMPTS) {
          failActiveConnection(
            `${failureMessage} 已停止本地等待并解锁输入；请重启服务后重新进入当前会话。`
          );
          return;
        }
        const delayMs = Math.min(
          SSE_RECONNECT_BASE_DELAY_MS * 2 ** (attempt - 1),
          SSE_RECONNECT_MAX_DELAY_MS
        );
        reconnectTimerRef.current = window.setTimeout(() => {
          reconnectTimerRef.current = null;
          // Check generation again before reconnecting
          if (streamGenerationRef.current === generation) {
            console.warn('session event stream reconnecting', {
              sessionId,
              attempt,
              delayMs,
              cursor: lastEventIdRef.current,
            });
            void startStream(lastEventIdRef.current);
          }
        }, delayMs);
      };

      const startStream = async (cursor: string | null): Promise<void> => {
        // Check if this connection is still active
        if (streamGenerationRef.current !== generation) {
          return;
        }

        const controller = new AbortController();
        streamAbortRef.current = controller;
        try {
          const response = await request(
            `/api/sessions/${encodeURIComponent(sessionId)}/events${buildSessionEventQueryString({
              afterEventId: cursor,
              filter: connectedSessionFilterRef.current,
            })}`,
            {
              headers: {
                Accept: 'text/event-stream',
                'Cache-Control': 'no-cache',
              },
              signal: controller.signal,
            }
          );

          // Check generation after request
          if (streamGenerationRef.current !== generation) {
            controller.abort();
            return;
          }

          reconnectAttemptRef.current = 0;
          const closeReason = await consumeSseStream(
            response,
            (payload, eventId) => {
              // Check generation before processing each event
              if (streamGenerationRef.current !== generation) {
                return;
              }
              if (eventId) {
                lastEventIdRef.current = eventId;
              }
              try {
                dispatchIncomingEvent(JSON.parse(payload));
              } catch (error) {
                dispatchIncomingEvent({
                  protocolVersion: 1,
                  event: 'error',
                  data: {
                    turnId: null,
                    code: 'invalid_agent_event',
                    message: String(error),
                  },
                });
              }
            },
            controller.signal
          );
          if (closeReason === 'ended') {
            // 会话事件流按设计应长期保持连接；如果无错误地 EOF，通常也是传输链路抖动。
            // 桌面端 WebView 偶发会触发这种“静默断流”，必须自动重连，否则 UI 会停在旧状态。
            if (
              !controller.signal.aborted &&
              connectedSessionIdRef.current === sessionId &&
              streamGenerationRef.current === generation
            ) {
              console.warn('session event stream ended unexpectedly, scheduling reconnect', {
                sessionId,
                cursor: lastEventIdRef.current,
              });
              scheduleReconnect('与服务端的事件流连接已中断。');
            }
            return;
          }
        } catch (error) {
          // Check generation before handling error
          if (streamGenerationRef.current !== generation) {
            return;
          }
          if (!controller.signal.aborted && connectedSessionIdRef.current === sessionId) {
            if (shouldRetryEventStream(error)) {
              scheduleReconnect(error instanceof Error ? error.message : '无法连接后端事件流。');
            } else {
              dispatchStreamError(
                onEventRef.current,
                error instanceof Error ? error.message : String(error)
              );
            }
          }
        } finally {
          if (streamAbortRef.current === controller) {
            streamAbortRef.current = null;
          }
        }
      };

      void startStream(lastEventIdRef.current);
    },
    [clearReconnectTimer, dispatchIncomingEvent, failActiveConnection]
  );

  const disconnectSession = useCallback(() => {
    clearReconnectTimer();
    streamAbortRef.current?.abort();
    streamAbortRef.current = null;
    connectedSessionIdRef.current = null;
    connectedSessionFilterRef.current = undefined;
    lastEventIdRef.current = null;
    reconnectAttemptRef.current = 0;
    // Increment generation to invalidate any pending operations
    streamGenerationRef.current++;
  }, [clearReconnectTimer]);

  const handleCreateSession = useCallback(async (workingDir: string): Promise<SessionMeta> => {
    return createSession(workingDir);
  }, []);

  const handleListSessionsWithMeta = useCallback(async (): Promise<SessionMeta[]> => {
    return listSessionsWithMeta();
  }, []);

  const handleLoadSession = useCallback(
    async (sessionId: string, filter?: SessionEventFilterQuery): Promise<SessionSnapshot> => {
      const { events, cursor, phase } = await loadSession(sessionId, filter);
      return { events, cursor, phase };
    },
    []
  );

  const handleSubmitPrompt = useCallback(
    async (sessionId: string, text: string): Promise<PromptSubmission> => {
      const response = await submitPrompt(sessionId, text);
      return response;
    },
    []
  );

  const handleInterrupt = useCallback(
    async (sessionId: string): Promise<void> => {
      try {
        await interruptSession(sessionId);
      } catch (error) {
        console.error('failed to interrupt session:', error);
        failActiveConnection(error instanceof Error ? error.message : String(error));
      }
    },
    [failActiveConnection]
  );

  const handleCompactSession = useCallback(async (sessionId: string): Promise<void> => {
    await compactSession(sessionId);
  }, []);

  const handleCancelSubRun = useCallback(
    async (sessionId: string, subRunId: string): Promise<void> => {
      try {
        await cancelSubRun(sessionId, subRunId);
      } catch (error) {
        console.error('failed to cancel sub-run:', error);
        throw error;
      }
    },
    []
  );

  const handleDeleteSession = useCallback(async (sessionId: string): Promise<void> => {
    await deleteSession(sessionId);
  }, []);

  const handleDeleteProject = useCallback(
    async (workingDir: string): Promise<DeleteProjectResult> => {
      return deleteProject(workingDir);
    },
    []
  );

  const handleListComposerOptions = useCallback(
    async (sessionId: string, query: string, signal?: AbortSignal): Promise<ComposerOption[]> => {
      return listComposerOptions(sessionId, query, signal);
    },
    []
  );

  const handleGetConfig = useCallback(async (): Promise<ConfigView> => {
    return getConfig();
  }, []);

  const handleSaveActiveSelection = useCallback(
    async (activeProfile: string, activeModel: string): Promise<void> => {
      await saveActiveSelection(activeProfile, activeModel);
    },
    []
  );

  const setModel = useCallback(
    async (profileName: string, model: string): Promise<void> => {
      await handleSaveActiveSelection(profileName, model);
    },
    [handleSaveActiveSelection]
  );

  const handleGetCurrentModel = useCallback(async (): Promise<CurrentModelInfo> => {
    return getCurrentModel();
  }, []);

  const handleListAvailableModels = useCallback(async (): Promise<ModelOption[]> => {
    return listAvailableModels();
  }, []);

  const handleTestConnection = useCallback(
    async (profileName: string, model: string): Promise<TestResult> => {
      return testConnection(profileName, model);
    },
    []
  );

  const openConfigInEditor = useCallback(async (path?: string): Promise<void> => {
    // 每次调用时即时获取最新桥接，避免组件初始化时 Tauri 环境未就绪导致拿到错误的 browserBridge
    await getHostBridge().openConfigInEditor(path);
  }, []);

  const selectDirectory = useCallback(async (): Promise<string | null> => {
    return getHostBridge().selectDirectory();
  }, []);

  return {
    createSession: handleCreateSession,
    listSessionsWithMeta: handleListSessionsWithMeta,
    loadSession: handleLoadSession,
    connectSession,
    disconnectSession,
    submitPrompt: handleSubmitPrompt,
    interrupt: handleInterrupt,
    cancelSubRun: handleCancelSubRun,
    compactSession: handleCompactSession,
    deleteSession: handleDeleteSession,
    deleteProject: handleDeleteProject,
    listComposerOptions: handleListComposerOptions,
    getConfig: handleGetConfig,
    saveActiveSelection: handleSaveActiveSelection,
    setModel,
    getCurrentModel: handleGetCurrentModel,
    listAvailableModels: handleListAvailableModels,
    testConnection: handleTestConnection,
    openConfigInEditor,
    selectDirectory,
    hostBridge: getHostBridge(),
  };
}
