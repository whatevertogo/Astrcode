import { useCallback, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { AgentEvent, AgentEventPayload } from '../types';

export function useAgent(onEvent: (event: AgentEventPayload) => void) {
  const onEventRef = useRef(onEvent);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listen<AgentEvent>('agent-event', (event) => {
      if (disposed) {
        return;
      }
      onEventRef.current(event.payload);
    })
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

  const submitPrompt = useCallback(async (text: string): Promise<void> => {
    await invoke('submit_prompt', { text });
  }, []);

  const interrupt = useCallback(async (): Promise<void> => {
    await invoke('interrupt');
  }, []);

  const getWorkingDir = useCallback(async (): Promise<string> => {
    return invoke<string>('get_working_dir');
  }, []);

  const exitApp = useCallback((): void => {
    void invoke('exit_app');
  }, []);

  return { submitPrompt, interrupt, getWorkingDir, exitApp };
}
