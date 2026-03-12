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
  success?: boolean;
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

async function consumeSseStream(
  response: Response,
  onMessage: (payload: string) => void,
  signal: AbortSignal
): Promise<void> {
  if (!response.body) {
    throw new Error('event stream response has no body');
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let dataLines: string[] = [];

  const flushEvent = () => {
    if (dataLines.length === 0) {
      return;
    }
    const payload = dataLines.join('\n');
    dataLines = [];
    onMessage(payload);
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
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }
  }

  buffer += decoder.decode();
  if (buffer) {
    const lines = buffer.split(/\r?\n/);
    for (const line of lines) {
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
  const connectedSessionIdRef = useRef<string | null>(null);
  const hostBridgeRef = useRef(getHostBridge());

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    return () => {
      streamAbortRef.current?.abort();
      streamAbortRef.current = null;
    };
  }, []);

  const dispatchIncomingEvent = useCallback((rawEvent: unknown) => {
    onEventRef.current(normalizeAgentEvent(rawEvent));
  }, []);

  const connectSession = useCallback(
    async (sessionId: string, afterEventId?: string | null): Promise<void> => {
      await ensureServerSession();
      streamAbortRef.current?.abort();
      const controller = new AbortController();
      streamAbortRef.current = controller;
      connectedSessionIdRef.current = sessionId;

      void (async () => {
        try {
          const response = await request(
            `/api/sessions/${encodeURIComponent(sessionId)}/events${
              afterEventId ? `?afterEventId=${encodeURIComponent(afterEventId)}` : ''
            }`,
            {
              headers: {
                Accept: 'text/event-stream',
                'Cache-Control': 'no-cache',
              },
              signal: controller.signal,
            }
          );
          await consumeSseStream(
            response,
            (payload) => {
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
          if (!controller.signal.aborted && streamAbortRef.current === controller) {
            dispatchStreamError(onEventRef.current, '事件流连接已关闭，请重新打开当前会话。');
          }
        } catch (error) {
          if (!controller.signal.aborted) {
            dispatchStreamError(
              onEventRef.current,
              error instanceof Error ? error.message : String(error)
            );
          }
        } finally {
          if (streamAbortRef.current === controller) {
            streamAbortRef.current = null;
          }
        }
      })();
    },
    [dispatchIncomingEvent]
  );

  const disconnectSession = useCallback(() => {
    streamAbortRef.current?.abort();
    streamAbortRef.current = null;
    connectedSessionIdRef.current = null;
  }, []);

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
