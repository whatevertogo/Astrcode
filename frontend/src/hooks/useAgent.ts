import { useCallback, useEffect, useRef } from 'react';
import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  AgentEventPayload,
  ConfigView,
  CurrentModelInfo,
  DeleteProjectResult,
  Message,
  ModelOption,
  SessionMeta,
  TestResult,
} from '../types';
import { isTauriEnvironment, waitForTauriEnvironment } from '../lib/tauri';
import { normalizeAgentEvent } from '../lib/agentEvent';

// ────────────────────────────────────────────────────────────
// Session message types (mirror Rust SessionMessage)
// ────────────────────────────────────────────────────────────

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

interface WebChatInputMessage {
  role: 'user' | 'assistant';
  content: string;
}

function normalizeSessionId(sessionId: string): string {
  const trimmed = sessionId.trim();
  if (!trimmed) {
    return '';
  }
  return trimmed.startsWith('session-') ? trimmed.slice('session-'.length) : trimmed;
}

function toWebChatMessages(messages: Message[], latestUserText: string): WebChatInputMessage[] {
  const history = messages.reduce<WebChatInputMessage[]>((acc, message) => {
    if (message.kind === 'user') {
      acc.push({ role: 'user', content: message.text });
      return acc;
    }
    if (message.kind === 'assistant' && message.text.trim()) {
      acc.push({ role: 'assistant', content: message.text });
    }
    return acc;
  }, []);

  history.push({ role: 'user', content: latestUserText });
  return history;
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
    // ignore json parse failure and fall back to status text
  }

  throw new Error(message);
}

async function streamWebChat(
  text: string,
  messages: Message[],
  onEvent: (rawEvent: unknown) => void,
  controller: AbortController
): Promise<void> {
  const turnId = `web-${Date.now()}`;
  const response = await fetch('/api/web-chat', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      turnId,
      messages: toWebChatMessages(messages, text),
    }),
    signal: controller.signal,
  });

  await ensureOk(response);

  if (!response.body) {
    throw new Error('Web 调试接口未返回可读取的数据流');
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  let doneReading = false;
  while (!doneReading) {
    const { value, done } = await reader.read();
    if (done) {
      doneReading = true;
      continue;
    }

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? '';

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) {
        continue;
      }
      onEvent(JSON.parse(trimmed));
    }
  }

  if (buffer.trim()) {
    onEvent(JSON.parse(buffer.trim()));
  }
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

function unsupportedDesktopFeature(name: string): Error {
  return new Error(`${name} 仅在桌面端可用`);
}

export function useAgent(onEvent: (event: AgentEventPayload) => void) {
  const onEventRef = useRef(onEvent);
  const webAbortControllerRef = useRef<AbortController | null>(null);
  const desktopDeltaRef = useRef<{
    turnId: string | null;
    text: string;
    frameId: number | null;
  }>({
    turnId: null,
    text: '',
    frameId: null,
  });

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    return () => {
      const pending = desktopDeltaRef.current;
      if (pending.frameId != null) {
        window.cancelAnimationFrame(pending.frameId);
        pending.frameId = null;
      }
    };
  }, []);

  const flushDesktopDelta = useCallback(() => {
    const pending = desktopDeltaRef.current;
    if (pending.frameId != null) {
      window.cancelAnimationFrame(pending.frameId);
      pending.frameId = null;
    }

    if (!pending.turnId || !pending.text) {
      pending.turnId = null;
      pending.text = '';
      return;
    }

    onEventRef.current({
      event: 'modelDelta',
      data: {
        turnId: pending.turnId,
        delta: pending.text,
      },
    });

    pending.turnId = null;
    pending.text = '';
  }, []);

  const scheduleDesktopDeltaFlush = useCallback(() => {
    const pending = desktopDeltaRef.current;
    if (pending.frameId != null) {
      return;
    }

    pending.frameId = window.requestAnimationFrame(() => {
      pending.frameId = null;
      flushDesktopDelta();
    });
  }, [flushDesktopDelta]);

  const dispatchIncomingEvent = useCallback((rawEvent: unknown) => {
    onEventRef.current(normalizeAgentEvent(rawEvent));
  }, []);

  const submitPrompt = useCallback(
    async (text: string, messages: Message[] = []): Promise<void> => {
      if (!isTauriEnvironment()) {
        webAbortControllerRef.current?.abort();
        const controller = new AbortController();
        webAbortControllerRef.current = controller;
        try {
          await streamWebChat(text, messages, dispatchIncomingEvent, controller);
        } catch (error) {
          if (!isAbortError(error)) {
            throw error;
          }
        } finally {
          if (webAbortControllerRef.current === controller) {
            webAbortControllerRef.current = null;
          }
        }
        return;
      }

      await waitForTauriEnvironment();
      flushDesktopDelta();

      const channel = new Channel<unknown>();

      channel.onmessage = (rawPayload) => {
        const payload = normalizeAgentEvent(rawPayload);

        if (payload.event === 'modelDelta') {
          const pending = desktopDeltaRef.current;
          if (pending.turnId && pending.turnId !== payload.data.turnId) {
            flushDesktopDelta();
          }
          pending.turnId = payload.data.turnId;
          pending.text += payload.data.delta;
          scheduleDesktopDeltaFlush();
          return;
        }

        if (payload.event === 'thinkingDelta') {
          flushDesktopDelta();
          onEventRef.current(payload);
          return;
        }

        flushDesktopDelta();
        onEventRef.current(payload);
      };

      await invoke('submit_prompt', { text, channel });
      flushDesktopDelta();
    },
    [dispatchIncomingEvent, flushDesktopDelta, scheduleDesktopDeltaFlush]
  );

  const interrupt = useCallback(async (): Promise<void> => {
    if (!isTauriEnvironment()) {
      webAbortControllerRef.current?.abort();
      webAbortControllerRef.current = null;
      onEventRef.current({
        event: 'phaseChanged',
        data: { phase: 'idle', turnId: null },
      });
      return;
    }

    await waitForTauriEnvironment();
    await invoke('interrupt');
  }, []);

  const getWorkingDir = useCallback(async (): Promise<string> => {
    if (!isTauriEnvironment()) {
      return 'Web Debug Mode';
    }
    try {
      await waitForTauriEnvironment();
      return invoke<string>('get_working_dir');
    } catch {
      return 'Web Debug Mode';
    }
  }, []);

  const exitApp = useCallback((): void => {
    if (!isTauriEnvironment()) {
      return;
    }

    void waitForTauriEnvironment().then(() => invoke('exit_app'));
  }, []);

  // ────────────────────────────────────────────────────────────
  // Session management
  // ────────────────────────────────────────────────────────────

  const listSessions = useCallback(async (): Promise<string[]> => {
    if (!isTauriEnvironment()) {
      return [];
    }
    try {
      await waitForTauriEnvironment();
      const result = await invoke<string[]>('list_sessions');
      const normalized = Array.from(new Set(result.map(normalizeSessionId).filter(Boolean)));
      console.log('[listSessions] Result:', normalized);
      return normalized;
    } catch (err) {
      console.log('[listSessions] Error:', err);
      return [];
    }
  }, []);

  const listSessionsWithMeta = useCallback(async (): Promise<SessionMeta[]> => {
    if (!isTauriEnvironment()) {
      return [];
    }

    try {
      await waitForTauriEnvironment();
      const result = await invoke<SessionMeta[]>('list_sessions_with_meta');
      console.log('[listSessionsWithMeta] Result:', result);
      return result;
    } catch (err) {
      console.error('[listSessionsWithMeta] Error:', err);
      return [];
    }
  }, []);

  const loadSession = useCallback(async (sessionId: string): Promise<SessionMessage[]> => {
    if (!isTauriEnvironment()) {
      return [];
    }
    try {
      const normalizedSessionId = normalizeSessionId(sessionId);
      if (!normalizedSessionId) {
        return [];
      }
      console.log('[loadSession] Loading session:', normalizedSessionId);
      await waitForTauriEnvironment();
      const result = await invoke<SessionMessage[]>('load_session', {
        sessionId: normalizedSessionId,
      });
      console.log('[loadSession] Result count:', result.length, result);
      return result;
    } catch (err) {
      console.log('[loadSession] Error:', err);
      return [];
    }
  }, []);

  const switchSession = useCallback(async (sessionId: string): Promise<string> => {
    if (!isTauriEnvironment()) {
      return normalizeSessionId(sessionId) || sessionId;
    }
    const normalizedSessionId = normalizeSessionId(sessionId);
    if (!normalizedSessionId) {
      throw new Error('invalid session id');
    }
    await waitForTauriEnvironment();
    try {
      const nextSessionId = await invoke<string>('switch_session', {
        sessionId: normalizedSessionId,
      });
      const normalizedNext = normalizeSessionId(nextSessionId);
      if (!normalizedNext) {
        throw new Error('backend returned empty session id');
      }
      return normalizedNext;
    } catch (error) {
      throw new Error(`switch session failed: ${String(error)}`);
    }
  }, []);

  const newSession = useCallback(async (): Promise<string> => {
    if (!isTauriEnvironment()) {
      return `web-${Date.now()}`;
    }
    try {
      await waitForTauriEnvironment();
      const nextSessionId = await invoke<string>('new_session');
      return normalizeSessionId(nextSessionId);
    } catch {
      return `web-${Date.now()}`;
    }
  }, []);

  const getSessionId = useCallback(async (): Promise<string> => {
    if (!isTauriEnvironment()) {
      return '';
    }
    try {
      await waitForTauriEnvironment();
      const result = await invoke<string>('get_session_id');
      const normalized = normalizeSessionId(result);
      console.log('[getSessionId] Result:', normalized);
      return normalized;
    } catch (err) {
      console.error('[getSessionId] Error:', err);
      return '';
    }
  }, []);

  const deleteSession = useCallback(async (sessionId: string): Promise<void> => {
    if (!isTauriEnvironment()) {
      return;
    }
    const normalizedSessionId = normalizeSessionId(sessionId);
    if (!normalizedSessionId) {
      return;
    }
    await waitForTauriEnvironment();
    await invoke('delete_session', { sessionId: normalizedSessionId });
  }, []);

  const deleteProject = useCallback(async (workingDir: string): Promise<DeleteProjectResult> => {
    if (!isTauriEnvironment()) {
      return { successCount: 0, failedSessionIds: [] };
    }

    await waitForTauriEnvironment();
    return invoke<DeleteProjectResult>('delete_project', { workingDir });
  }, []);

  const getConfig = useCallback(async (): Promise<ConfigView> => {
    if (!isTauriEnvironment()) {
      throw unsupportedDesktopFeature('配置读取');
    }
    await waitForTauriEnvironment();
    return invoke<ConfigView>('get_config');
  }, []);

  const saveActiveSelection = useCallback(
    async (activeProfile: string, activeModel: string): Promise<void> => {
      if (!isTauriEnvironment()) {
        throw unsupportedDesktopFeature('配置保存');
      }
      await waitForTauriEnvironment();
      await invoke('save_active_selection', { activeProfile, activeModel });
    },
    []
  );

  const testConnection = useCallback(
    async (profileName: string, model: string): Promise<TestResult> => {
      if (!isTauriEnvironment()) {
        throw unsupportedDesktopFeature('连接测试');
      }
      await waitForTauriEnvironment();
      return invoke<TestResult>('test_connection', { profileName, model });
    },
    []
  );

  const openConfigInEditor = useCallback(async (): Promise<void> => {
    if (!isTauriEnvironment()) {
      throw unsupportedDesktopFeature('打开配置文件');
    }
    await waitForTauriEnvironment();
    await invoke('open_config_in_editor');
  }, []);

  const setModel = useCallback(async (profileName: string, model: string): Promise<void> => {
    if (!isTauriEnvironment()) {
      throw unsupportedDesktopFeature('模型切换');
    }
    await waitForTauriEnvironment();
    await invoke('set_model', { profileName, model });
  }, []);

  const getCurrentModel = useCallback(async (): Promise<CurrentModelInfo> => {
    if (!isTauriEnvironment()) {
      throw unsupportedDesktopFeature('当前模型读取');
    }
    await waitForTauriEnvironment();
    return invoke<CurrentModelInfo>('get_current_model');
  }, []);

  const listAvailableModels = useCallback(async (): Promise<ModelOption[]> => {
    if (!isTauriEnvironment()) {
      throw unsupportedDesktopFeature('模型列表读取');
    }
    await waitForTauriEnvironment();
    return invoke<ModelOption[]>('list_available_models');
  }, []);

  return {
    submitPrompt,
    interrupt,
    getWorkingDir,
    exitApp,
    listSessions,
    listSessionsWithMeta,
    loadSession,
    switchSession,
    newSession,
    getSessionId,
    deleteSession,
    deleteProject,
    getConfig,
    saveActiveSelection,
    setModel,
    getCurrentModel,
    listAvailableModels,
    testConnection,
    openConfigInEditor,
  };
}
