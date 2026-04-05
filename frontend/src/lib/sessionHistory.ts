import { reducer } from '../store/reducer';
import type { AgentEventPayload, AppState, Message, Phase } from '../types';
import { applyAgentEvent } from './applyAgentEvent';

interface SessionHistoryReplayResult {
  messages: Message[];
  phase: Phase;
}

/// 用历史 AgentEvent 重建单个 session 的前端视图。
///
/// 这里故意复用实时事件的分发规则，而不是再引入 `SessionMessage` 专用投影。
/// `phase` 使用服务端单独返回的当前值做最终兜底，因为内存中活跃会话的 phase
/// 可能比最后一条已持久化 phase 事件更新。
export function replaySessionHistory(
  sessionId: string,
  events: AgentEventPayload[],
  currentPhase: Phase
): SessionHistoryReplayResult {
  let state: AppState = {
    projects: [
      {
        id: '__history_project__',
        name: 'history',
        workingDir: '',
        isExpanded: true,
        sessions: [
          {
            id: sessionId,
            projectId: '__history_project__',
            title: '新会话',
            createdAt: 0,
            messages: [],
          },
        ],
      },
    ],
    activeProjectId: '__history_project__',
    activeSessionId: sessionId,
    phase: 'idle',
  };

  const activeSessionIdRef = { current: sessionId as string | null };
  const pendingSubmitSessionRef = { current: [] as string[] };
  const turnSessionMapRef = { current: {} as Record<string, string> };
  const phaseRef = { current: 'idle' as Phase };
  const dispatch = (action: Parameters<typeof reducer>[1]) => {
    state = reducer(state, action);
  };

  for (const event of events) {
    applyAgentEvent(
      {
        activeSessionIdRef,
        pendingSubmitSessionRef,
        turnSessionMapRef,
        phaseRef,
        dispatch,
      },
      event
    );
  }

  state = reducer(state, { type: 'SET_PHASE', phase: currentPhase });

  return {
    messages: state.projects[0]?.sessions[0]?.messages ?? [],
    phase: currentPhase,
  };
}
