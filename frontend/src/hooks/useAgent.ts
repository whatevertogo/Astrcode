import { useCallback, useEffect, useRef } from 'react';
import type {
  AgentEventPayload,
  ConfigView,
  CurrentModelInfo,
  DeleteProjectResult,
  ModelOption,
  SessionMeta,
  TestResult,
} from '../types';
import { normalizeAgentEvent } from '../lib/agentEvent';
import { getHostBridge } from '../lib/hostBridge';
import { ensureServerSession, getServerAuthToken, getServerOrigin } from '../lib/serverAuth';

export interface SessionUserMessage {
  kind: 'user';
  content: string;
  timestamp: string;
}

export interface SessionAssistantMessage {
  kind: 'assistant';
  content: string;
  timestamp: string;
}

export interface SessionToolCallMessage {
  kind: 'toolCall';
  toolCallId: string;
  toolName: string;
  args: unknown;
  output?: string;
  ok?: boolean;
  durationMs?: number;
}

export type SessionMessage = SessionUserMessage | SessionAssistantMessage | SessionToolCallMessage;

export interface SessionSnapshot {
  messages: SessionMessage[];
  cursor: string | null;
}

function buildAuthHeaders(headers?: HeadersInit): Headers {
  const merged = new Headers(headers);
  const token = getServerAuthToken();
  if (token) {
    merged.set('x-astrcode-token', token);
  }
  return merged;
}

async function ensureOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  let message = `${response.status} ${response.statusText}`;
  try {
    const payload = (await response.json()) as { error?: unknown };
    if (typeof payload.error === 'string' && payload.error) {
      message = payload.error;
    }
  } catch {
    // ignore
  }

  throw new Error(message);
}

function normalizeFetchError(error: unknown): Error {
  if (error instanceof Error && error.name === 'AbortError') {
    return error;
  }

  if (error instanceof TypeError) {
    if (window.__ASTRCODE_BOOTSTRAP__?.isDesktopHost) {
      return new Error(
        '无法连接本地服务，请确认 AstrCode 桌面端仍在运行；如果刚关闭了启动它的终端，请重新执行 `cargo tauri dev`。'
      );
    }
    return new Error('无法连接后端服务，请确认本地 server 或网络连接正常。');
  }

  return error instanceof Error ? error : new Error(String(error));
}

async function requestResponse(path: string, init?: RequestInit): Promise<Response> {
  await ensureServerSession();
  try {
    return await fetch(`${getServerOrigin()}${path}`, {
      ...init,
      headers: buildAuthHeaders(init?.headers),
    });
  } catch (error) {
    throw normalizeFetchError(error);
  }
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await requestResponse(path, init);
  await ensureOk(response);
  return (await response.json()) as T;
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  const response = await requestResponse(path, init);
  await ensureOk(response);
  return response;
}

const SSE_RECONNECT_BASE_DELAY_MS = 500;
const SSE_RECONNECT_MAX_DELAY_MS = 5_000;

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

async function consumeSseStream(
  response: Response,
  onMessage: (payload: string, eventId: string | null) => void,
  signal: AbortSignal
): Promise<void> {
  if (!response.body) {
    throw new Error('event stream response has no body');
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let dataLines: string[] = [];
  let eventId: string | null = null;

  const flushEvent = () => {
    if (dataLines.length === 0) {
      return;
    }
    const payload = dataLines.join('\n');
    dataLines = [];
    onMessage(payload, eventId);
    eventId = null;
  };

  while (!signal.aborted) {
    const { value, done } = await reader.read();
    if (done) {
      break;
    }

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? '';

    for (const line of lines) {
      if (line === '') {
        flushEvent();
        continue;
      }
      if (line.startsWith(':')) {
        continue;
      }
      if (line.startsWith('id:')) {
        eventId = line.slice(3).trimStart();
        continue;
      }
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }
  }

  buffer += decoder.decode();
  if (buffer) {
    const lines = buffer.split(/\r?\n/);
    for (const line of lines) {
      if (line.startsWith('id:')) {
        eventId = line.slice(3).trimStart();
        continue;
      }
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }
  }
  flushEvent();
}

export function useAgent(onEvent: (event: AgentEventPayload) => void) {
  const onEventRef = useRef(onEvent);
  const streamAbortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const reconnectAttemptRef = useRef(0);
  const connectedSessionIdRef = useRef<string | null>(null);
  const lastEventIdRef = useRef<string | null>(null);
  const hostBridgeRef = useRef(getHostBridge());

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

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
    async (sessionId: string, afterEventId?: string | null): Promise<void> => {
      await ensureServerSession();
      clearReconnectTimer();
      streamAbortRef.current?.abort();
      connectedSessionIdRef.current = sessionId;
      lastEventIdRef.current = afterEventId ?? null;
      reconnectAttemptRef.current = 0;

      const scheduleReconnect = () => {
        if (connectedSessionIdRef.current !== sessionId) {
          return;
        }
        clearReconnectTimer();
        const attempt = reconnectAttemptRef.current + 1;
        reconnectAttemptRef.current = attempt;
        const delayMs = Math.min(
          SSE_RECONNECT_BASE_DELAY_MS * 2 ** (attempt - 1),
          SSE_RECONNECT_MAX_DELAY_MS
        );
        reconnectTimerRef.current = window.setTimeout(() => {
          reconnectTimerRef.current = null;
          void startStream(lastEventIdRef.current);
        }, delayMs);
      };

      const startStream = async (cursor: string | null): Promise<void> => {
        const controller = new AbortController();
        streamAbortRef.current = controller;
        try {
          const response = await request(
            `/api/sessions/${encodeURIComponent(sessionId)}/events${
              cursor ? `?afterEventId=${encodeURIComponent(cursor)}` : ''
            }`,
            {
              headers: {
                Accept: 'text/event-stream',
                'Cache-Control': 'no-cache',
              },
              signal: controller.signal,
            }
          );
          reconnectAttemptRef.current = 0;
          await consumeSseStream(
            response,
            (payload, eventId) => {
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
          if (
            !controller.signal.aborted &&
            streamAbortRef.current === controller &&
            connectedSessionIdRef.current === sessionId
          ) {
            scheduleReconnect();
          }
        } catch (error) {
          if (!controller.signal.aborted && connectedSessionIdRef.current === sessionId) {
            if (shouldRetryEventStream(error)) {
              scheduleReconnect();
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
    [clearReconnectTimer, dispatchIncomingEvent]
  );

  const disconnectSession = useCallback(() => {
    clearReconnectTimer();
    streamAbortRef.current?.abort();
    streamAbortRef.current = null;
    connectedSessionIdRef.current = null;
    lastEventIdRef.current = null;
    reconnectAttemptRef.current = 0;
  }, [clearReconnectTimer]);

  const createSession = useCallback(async (workingDir: string): Promise<SessionMeta> => {
    return requestJson<SessionMeta>('/api/sessions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ workingDir }),
    });
  }, []);

  const listSessionsWithMeta = useCallback(async (): Promise<SessionMeta[]> => {
    return requestJson<SessionMeta[]>('/api/sessions');
  }, []);

  const loadSession = useCallback(async (sessionId: string): Promise<SessionSnapshot> => {
    const response = await request(`/api/sessions/${encodeURIComponent(sessionId)}/messages`);
    const messages = (await response.json()) as SessionMessage[];
    const cursor = response.headers.get('x-session-cursor');
    return { messages, cursor };
  }, []);

  const submitPrompt = useCallback(async (sessionId: string, text: string): Promise<string> => {
    const response = await requestJson<{ turnId: string }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/prompts`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      }
    );
    return response.turnId;
  }, []);

  const interrupt = useCallback(async (sessionId: string): Promise<void> => {
    await request(`/api/sessions/${encodeURIComponent(sessionId)}/interrupt`, {
      method: 'POST',
    });
  }, []);

  const deleteSession = useCallback(async (sessionId: string): Promise<void> => {
    await request(`/api/sessions/${encodeURIComponent(sessionId)}`, {
      method: 'DELETE',
    });
  }, []);

  const deleteProject = useCallback(async (workingDir: string): Promise<DeleteProjectResult> => {
    return requestJson<DeleteProjectResult>(
      `/api/projects?workingDir=${encodeURIComponent(workingDir)}`,
      {
        method: 'DELETE',
      }
    );
  }, []);

  const getConfig = useCallback(async (): Promise<ConfigView> => {
    return requestJson<ConfigView>('/api/config');
  }, []);

  const saveActiveSelection = useCallback(
    async (activeProfile: string, activeModel: string): Promise<void> => {
      await request('/api/config/active-selection', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ activeProfile, activeModel }),
      });
    },
    []
  );

  const setModel = useCallback(
    async (profileName: string, model: string): Promise<void> => {
      await saveActiveSelection(profileName, model);
    },
    [saveActiveSelection]
  );

  const getCurrentModel = useCallback(async (): Promise<CurrentModelInfo> => {
    return requestJson<CurrentModelInfo>('/api/models/current');
  }, []);

  const listAvailableModels = useCallback(async (): Promise<ModelOption[]> => {
    return requestJson<ModelOption[]>('/api/models');
  }, []);

  const testConnection = useCallback(
    async (profileName: string, model: string): Promise<TestResult> => {
      return requestJson<TestResult>('/api/models/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ profileName, model }),
      });
    },
    []
  );

  const openConfigInEditor = useCallback(async (path?: string): Promise<void> => {
    await hostBridgeRef.current.openConfigInEditor(path);
  }, []);

  const selectDirectory = useCallback(async (): Promise<string | null> => {
    return hostBridgeRef.current.selectDirectory();
  }, []);

  return {
    createSession,
    listSessionsWithMeta,
    loadSession,
    connectSession,
    disconnectSession,
    submitPrompt,
    interrupt,
    deleteSession,
    deleteProject,
    getConfig,
    saveActiveSelection,
    setModel,
    getCurrentModel,
    listAvailableModels,
    testConnection,
    openConfigInEditor,
    selectDirectory,
    hostBridge: hostBridgeRef.current,
  };
}
