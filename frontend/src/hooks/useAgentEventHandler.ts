import { useCallback, type Dispatch, type MutableRefObject } from 'react';
import type { AgentEventPayload, Action, Phase } from '../types';
import { applyAgentEvent } from '../lib/applyAgentEvent';

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
      applyAgentEvent(
        {
          activeSessionIdRef,
          pendingSubmitSessionRef,
          turnSessionMapRef,
          phaseRef,
          dispatch,
          scheduleMicrotask: queueMicrotask,
        },
        event
      );
    },
    [activeSessionIdRef, dispatch, pendingSubmitSessionRef, phaseRef, turnSessionMapRef]
  );
}
