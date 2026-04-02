import { useCallback, useEffect, useRef } from 'react';

import type { SessionCatalogEventPayload } from '../types';
import { request } from '../lib/api/client';
import { normalizeSessionCatalogEvent } from '../lib/sessionCatalogEvent';
import { consumeSseStream } from '../lib/sse/consumer';
import { ensureServerSession } from '../lib/serverAuth';

const RECONNECT_BASE_DELAY_MS = 500;
const RECONNECT_MAX_DELAY_MS = 5_000;

function shouldRetrySessionCatalogStream(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();
  return !message.includes('unauthorized') && !message.includes('403');
}

interface UseSessionCatalogEventsOptions {
  onEvent: (event: SessionCatalogEventPayload) => void;
  onResync: () => void;
}

export function useSessionCatalogEvents({ onEvent, onResync }: UseSessionCatalogEventsOptions) {
  const onEventRef = useRef(onEvent);
  const onResyncRef = useRef(onResync);
  const abortRef = useRef<AbortController | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const reconnectAttemptRef = useRef(0);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    onResyncRef.current = onResync;
  }, [onResync]);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const connect = useCallback(async () => {
    await ensureServerSession();
    clearReconnectTimer();
    abortRef.current?.abort();

    const scheduleReconnect = () => {
      clearReconnectTimer();
      const attempt = reconnectAttemptRef.current + 1;
      reconnectAttemptRef.current = attempt;
      const delayMs = Math.min(
        RECONNECT_BASE_DELAY_MS * 2 ** (attempt - 1),
        RECONNECT_MAX_DELAY_MS
      );
      reconnectTimerRef.current = window.setTimeout(() => {
        reconnectTimerRef.current = null;
        void connect();
      }, delayMs);
    };

    const controller = new AbortController();
    abortRef.current = controller;

    try {
      const response = await request('/api/session-events', {
        headers: {
          Accept: 'text/event-stream',
          'Cache-Control': 'no-cache',
        },
        signal: controller.signal,
      });

      reconnectAttemptRef.current = 0;
      // 目录事件流不支持断点续传，因此每次重连成功后都主动做一次 resync。
      onResyncRef.current();

      await consumeSseStream(
        response,
        (payload) => {
          try {
            onEventRef.current(normalizeSessionCatalogEvent(JSON.parse(payload)));
          } catch (error) {
            console.error('Failed to process session catalog event:', error);
          }
        },
        controller.signal
      );

      if (!controller.signal.aborted) {
        scheduleReconnect();
      }
    } catch (error) {
      if (!controller.signal.aborted && shouldRetrySessionCatalogStream(error)) {
        scheduleReconnect();
      } else if (!controller.signal.aborted) {
        console.error('Session catalog event stream failed:', error);
      }
    } finally {
      if (abortRef.current === controller) {
        abortRef.current = null;
      }
    }
  }, [clearReconnectTimer]);

  const disconnect = useCallback(() => {
    clearReconnectTimer();
    abortRef.current?.abort();
    abortRef.current = null;
    reconnectAttemptRef.current = 0;
  }, [clearReconnectTimer]);

  useEffect(() => {
    void connect();
    return () => {
      disconnect();
    };
  }, [connect, disconnect]);
}
