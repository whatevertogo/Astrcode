//! # Agent Hook
//!
//! Orchestrates API calls and authoritative conversation streaming.

import { useCallback, useEffect, useRef } from 'react';
import { getHostBridge } from '../lib/hostBridge';
import { consumeSseStream } from '../lib/sse/consumer';
import { normalizeSessionIdForCompare } from '../lib/sessionId';
import { ensureServerSession } from '../lib/serverAuth';
import { request } from '../lib/api/client';
import { logger } from '../lib/logger';
import { listComposerOptions } from '../lib/api/composer';
import {
  applyConversationEnvelope,
  createConversationStreamRequestPath,
  loadConversationSnapshotState,
  projectConversationState,
  type ConversationSnapshotState,
  type ConversationViewProjection,
} from '../lib/api/conversation';
import {
  compactSession,
  closeChildAgent,
  createSession,
  deleteProject,
  deleteSession,
  forkSession,
  getSessionMode,
  interruptSession,
  listSessionsWithMeta,
  submitPrompt,
  switchSessionMode,
} from '../lib/api/sessions';
import type { PromptSubmission } from '../lib/api/sessions';
import { getConfig, reloadConfig, saveActiveSelection } from '../lib/api/config';
import { getCurrentModel, listAvailableModels, testConnection } from '../lib/api/models';
import type {
  ComposerOption,
  ConfigView,
  ConversationStepProgress,
  CurrentModelInfo,
  DeleteProjectResult,
  ExecutionControl,
  ModelOption,
  SessionModeState,
  SessionMeta,
  TestResult,
} from '../types';
import type { SessionEventFilterQuery } from '../lib/sessionView';

const SSE_RECONNECT_BASE_DELAY_MS = 500;
const SSE_RECONNECT_MAX_DELAY_MS = 5_000;
const SSE_RECONNECT_FATAL_ATTEMPTS = 3;

function isRehydrateRequiredEnvelope(payload: unknown): boolean {
  if (!payload || typeof payload !== 'object') {
    return false;
  }
  return (payload as { kind?: unknown }).kind === 'rehydrate_required';
}

export function processConversationStreamEnvelope(
  conversationState: ConversationSnapshotState,
  payload: string,
  filter?: SessionEventFilterQuery,
  messageTree?: ConversationViewProjection['messageTree']
):
  | {
      kind: 'projection';
      projection: ConversationViewProjection;
    }
  | {
      kind: 'rehydrate_required';
    } {
  const envelope: unknown = JSON.parse(payload);
  if (isRehydrateRequiredEnvelope(envelope)) {
    return { kind: 'rehydrate_required' };
  }
  applyConversationEnvelope(conversationState, envelope);
  return {
    kind: 'projection',
    projection: projectConversationState(conversationState, filter?.subRunId, messageTree),
  };
}

function shouldRetryEventStream(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();
  return !message.includes('unauthorized') && !message.includes('403');
}

function projectionSignature(projection: ConversationViewProjection): string {
  return `${projection.phase}::${projection.messageFingerprint}::${projection.childFingerprint}::${stepProgressSignature(projection.stepProgress)}`;
}

function stepProgressSignature(stepProgress: ConversationStepProgress): string {
  const fingerprintOf = (cursor: ConversationStepProgress['durable']): string =>
    cursor ? `${cursor.turnId}:${cursor.stepIndex}` : 'none';
  return `${fingerprintOf(stepProgress.durable)}|${fingerprintOf(stepProgress.live)}`;
}

export function useAgent() {
  const streamAbortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const frameFlushRef = useRef<number | null>(null);
  const reconnectAttemptRef = useRef(0);
  const connectedSessionIdRef = useRef<string | null>(null);
  const connectedSessionFilterRef = useRef<SessionEventFilterQuery | undefined>(undefined);
  const lastEventIdRef = useRef<string | null>(null);
  const conversationStateRef = useRef<ConversationSnapshotState | null>(null);
  const messageTreeRef = useRef<ConversationViewProjection['messageTree'] | null>(null);
  const projectionHandlerRef = useRef<((projection: ConversationViewProjection) => void) | null>(
    null
  );
  const pendingProjectionRef = useRef<ConversationViewProjection | null>(null);
  const lastProjectionSignatureRef = useRef<string | null>(null);
  const streamGenerationRef = useRef(0);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const flushProjectedConversation = useCallback(() => {
    if (frameFlushRef.current !== null) {
      window.cancelAnimationFrame(frameFlushRef.current);
      frameFlushRef.current = null;
    }
    const projection = pendingProjectionRef.current;
    pendingProjectionRef.current = null;
    if (!projection) {
      return;
    }
    projectionHandlerRef.current?.(projection);
  }, []);

  const queueProjectedConversation = useCallback(
    (projection: ConversationViewProjection) => {
      const signature = projectionSignature(projection);
      if (lastProjectionSignatureRef.current === signature) {
        return;
      }
      lastProjectionSignatureRef.current = signature;
      pendingProjectionRef.current = projection;
      if (frameFlushRef.current !== null) {
        return;
      }
      frameFlushRef.current = window.requestAnimationFrame(() => {
        frameFlushRef.current = null;
        flushProjectedConversation();
      });
    },
    [flushProjectedConversation]
  );

  const recoverConversationProjection = useCallback(
    async (sessionId: string, filter?: SessionEventFilterQuery): Promise<void> => {
      try {
        const snapshotState = await loadConversationSnapshotState(sessionId, filter);
        conversationStateRef.current = snapshotState;
        const projection = projectConversationState(snapshotState, filter?.subRunId);
        messageTreeRef.current = projection.messageTree;
        queueProjectedConversation(projection);
      } catch (error) {
        logger.warn('useAgent', 'failed to recover conversation projection from server snapshot', {
          sessionId,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [queueProjectedConversation]
  );

  useEffect(() => {
    return () => {
      streamAbortRef.current?.abort();
      streamAbortRef.current = null;
      clearReconnectTimer();
      if (frameFlushRef.current !== null) {
        window.cancelAnimationFrame(frameFlushRef.current);
        frameFlushRef.current = null;
      }
      pendingProjectionRef.current = null;
    };
  }, [clearReconnectTimer]);

  const failActiveConnection = useCallback(
    (message: string) => {
      const activeSessionId = connectedSessionIdRef.current;
      const activeFilter = connectedSessionFilterRef.current;
      logger.warn('useAgent', 'conversation stream stopped and unlocked input', {
        sessionId: activeSessionId,
        message,
      });
      clearReconnectTimer();
      streamAbortRef.current?.abort();
      streamAbortRef.current = null;
      connectedSessionIdRef.current = null;
      connectedSessionFilterRef.current = undefined;
      lastEventIdRef.current = null;
      conversationStateRef.current = null;
      messageTreeRef.current = null;
      projectionHandlerRef.current = null;
      pendingProjectionRef.current = null;
      reconnectAttemptRef.current = 0;
      streamGenerationRef.current += 1;
      flushProjectedConversation();
      if (activeSessionId) {
        void recoverConversationProjection(activeSessionId, activeFilter);
      }
    },
    [clearReconnectTimer, flushProjectedConversation, recoverConversationProjection]
  );

  const connectSession = useCallback(
    async (
      sessionId: string,
      afterEventId?: string | null,
      filter?: SessionEventFilterQuery,
      onProjection?: (projection: ConversationViewProjection) => void
    ): Promise<void> => {
      await ensureServerSession();
      clearReconnectTimer();
      streamAbortRef.current?.abort();
      pendingProjectionRef.current = null;

      const generation = ++streamGenerationRef.current;
      projectionHandlerRef.current = onProjection ?? null;
      connectedSessionIdRef.current = sessionId;
      connectedSessionFilterRef.current = filter;
      lastEventIdRef.current = afterEventId ?? null;
      reconnectAttemptRef.current = 0;

      const scheduleReconnect = (failureMessage: string) => {
        if (
          streamGenerationRef.current !== generation ||
          connectedSessionIdRef.current !== sessionId
        ) {
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
          if (streamGenerationRef.current === generation) {
            logger.warn('useAgent', 'conversation stream reconnecting', {
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
        if (streamGenerationRef.current !== generation) {
          return;
        }

        const controller = new AbortController();
        streamAbortRef.current = controller;
        try {
          const response = await request(
            createConversationStreamRequestPath(
              sessionId,
              cursor,
              connectedSessionFilterRef.current
            ),
            {
              headers: {
                Accept: 'text/event-stream',
                'Cache-Control': 'no-cache',
              },
              signal: controller.signal,
            }
          );

          if (streamGenerationRef.current !== generation) {
            controller.abort();
            return;
          }

          reconnectAttemptRef.current = 0;
          const closeReason = await consumeSseStream(
            response,
            (payload, eventId) => {
              if (streamGenerationRef.current !== generation) {
                return;
              }
              if (eventId) {
                lastEventIdRef.current = eventId;
              }
              try {
                const conversationState = conversationStateRef.current;
                if (!conversationState) {
                  return;
                }
                const result = processConversationStreamEnvelope(
                  conversationState,
                  payload,
                  connectedSessionFilterRef.current,
                  messageTreeRef.current ?? undefined
                );
                if (result.kind === 'rehydrate_required') {
                  void recoverConversationProjection(sessionId, connectedSessionFilterRef.current);
                  return;
                }
                const projection = result.projection;
                // TODO(stream-backpressure): 如果服务端开始做时间窗 coalescing，这里可以继续保留
                // “单帧只提交最后一个 projection” 的策略，避免高频 delta 把主线程重新打满。
                messageTreeRef.current = projection.messageTree;
                queueProjectedConversation(projection);
              } catch (error) {
                logger.warn('useAgent', 'invalid conversation stream envelope', {
                  sessionId,
                  error: error instanceof Error ? error.message : String(error),
                });
              }
            },
            controller.signal
          );

          if (closeReason === 'ended') {
            if (
              !controller.signal.aborted &&
              connectedSessionIdRef.current === sessionId &&
              streamGenerationRef.current === generation
            ) {
              logger.warn(
                'useAgent',
                'conversation stream ended unexpectedly, scheduling reconnect',
                {
                  sessionId,
                  cursor: lastEventIdRef.current,
                }
              );
              scheduleReconnect('与服务端的会话流连接已中断。');
            }
            return;
          }
          flushProjectedConversation();
        } catch (error) {
          if (streamGenerationRef.current !== generation) {
            return;
          }
          if (!controller.signal.aborted && connectedSessionIdRef.current === sessionId) {
            if (shouldRetryEventStream(error)) {
              scheduleReconnect(error instanceof Error ? error.message : '无法连接后端会话流。');
            } else {
              failActiveConnection(error instanceof Error ? error.message : String(error));
            }
          }
        } finally {
          flushProjectedConversation();
          if (streamAbortRef.current === controller) {
            streamAbortRef.current = null;
          }
        }
      };

      void startStream(lastEventIdRef.current);
    },
    [
      clearReconnectTimer,
      failActiveConnection,
      flushProjectedConversation,
      queueProjectedConversation,
      recoverConversationProjection,
    ]
  );

  const disconnectSession = useCallback(() => {
    clearReconnectTimer();
    streamAbortRef.current?.abort();
    streamAbortRef.current = null;
    connectedSessionIdRef.current = null;
    connectedSessionFilterRef.current = undefined;
    lastEventIdRef.current = null;
    conversationStateRef.current = null;
    messageTreeRef.current = null;
    projectionHandlerRef.current = null;
    pendingProjectionRef.current = null;
    reconnectAttemptRef.current = 0;
    lastProjectionSignatureRef.current = null;
    flushProjectedConversation();
    streamGenerationRef.current++;
  }, [clearReconnectTimer, flushProjectedConversation]);

  const handleCreateSession = useCallback(async (workingDir: string): Promise<SessionMeta> => {
    return createSession(workingDir);
  }, []);

  const handleListSessionsWithMeta = useCallback(async (): Promise<SessionMeta[]> => {
    return listSessionsWithMeta();
  }, []);

  const handleForkSession = useCallback(
    async (
      sessionId: string,
      options?: { turnId?: string; storageSeq?: number }
    ): Promise<SessionMeta> => {
      return forkSession(sessionId, options);
    },
    []
  );

  const handleLoadConversationView = useCallback(
    async (
      sessionId: string,
      filter?: SessionEventFilterQuery
    ): Promise<ConversationViewProjection> => {
      const snapshotState = await loadConversationSnapshotState(sessionId, filter);
      conversationStateRef.current = snapshotState;
      const projection = projectConversationState(snapshotState, filter?.subRunId);
      messageTreeRef.current = projection.messageTree;
      lastProjectionSignatureRef.current = projectionSignature(projection);
      return projection;
    },
    []
  );

  const handleSubmitPrompt = useCallback(
    async (sessionId: string, text: string): Promise<PromptSubmission> => {
      return submitPrompt(sessionId, text);
    },
    []
  );

  const handleInterrupt = useCallback(
    async (sessionId: string): Promise<void> => {
      try {
        await interruptSession(sessionId);
        if (connectedSessionIdRef.current === sessionId) {
          await recoverConversationProjection(sessionId, connectedSessionFilterRef.current);
        }
      } catch (error) {
        logger.error('useAgent', 'failed to interrupt session:', error);
        failActiveConnection(error instanceof Error ? error.message : String(error));
      }
    },
    [failActiveConnection, recoverConversationProjection]
  );

  const handleCompactSession = useCallback(
    async (
      sessionId: string,
      control?: ExecutionControl,
      instructions?: string
    ): Promise<{ accepted: boolean; deferred: boolean; message: string }> => {
      return compactSession(sessionId, control, instructions);
    },
    []
  );

  const handleGetSessionMode = useCallback(async (sessionId: string): Promise<SessionModeState> => {
    return getSessionMode(sessionId);
  }, []);

  const handleSwitchSessionMode = useCallback(
    async (sessionId: string, modeId: string): Promise<SessionModeState> => {
      return switchSessionMode(sessionId, modeId);
    },
    []
  );

  const handleCancelSubRun = useCallback(
    async (sessionId: string, agentId: string): Promise<void> => {
      try {
        await closeChildAgent(sessionId, agentId);
      } catch (error) {
        logger.error('useAgent', 'failed to close agent:', error);
        throw error;
      }
    },
    []
  );

  const handleDeleteSession = useCallback(
    async (sessionId: string): Promise<void> => {
      const activeSessionId = connectedSessionIdRef.current;
      if (
        activeSessionId &&
        normalizeSessionIdForCompare(activeSessionId) === normalizeSessionIdForCompare(sessionId)
      ) {
        disconnectSession();
      }
      await deleteSession(sessionId);
    },
    [disconnectSession]
  );

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

  const handleReloadConfig = useCallback(async (): Promise<void> => {
    await reloadConfig();
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
    await getHostBridge().openConfigInEditor(path);
  }, []);

  const selectDirectory = useCallback(async (): Promise<string | null> => {
    return getHostBridge().selectDirectory();
  }, []);

  return {
    createSession: handleCreateSession,
    forkSession: handleForkSession,
    listSessionsWithMeta: handleListSessionsWithMeta,
    loadConversationView: handleLoadConversationView,
    connectSession,
    disconnectSession,
    submitPrompt: handleSubmitPrompt,
    interrupt: handleInterrupt,
    cancelSubRun: handleCancelSubRun,
    compactSession: handleCompactSession,
    getSessionMode: handleGetSessionMode,
    switchSessionMode: handleSwitchSessionMode,
    deleteSession: handleDeleteSession,
    deleteProject: handleDeleteProject,
    listComposerOptions: handleListComposerOptions,
    getConfig: handleGetConfig,
    reloadConfig: handleReloadConfig,
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
