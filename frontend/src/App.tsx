import React, { useCallback, useEffect, useReducer, useRef } from 'react';
import type {
  AgentEventPayload,
  Action,
  AppState,
  Project,
  Session,
} from './types';
import { uuid } from './utils/uuid';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import { useAgent } from './hooks/useAgent';
import { useProjects } from './hooks/useProjects';

function getDirectoryName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || '默认项目';
}

// ────────────────────────────────────────────────────────────
// Helper: map a single project
// ────────────────────────────────────────────────────────────
function mapProject(
  state: AppState,
  projectId: string,
  fn: (p: Project) => Project,
): AppState {
  return {
    ...state,
    projects: state.projects.map((p) => (p.id === projectId ? fn(p) : p)),
  };
}

// ────────────────────────────────────────────────────────────
// Helper: map a single session (across all projects)
// ────────────────────────────────────────────────────────────
function mapSession(
  state: AppState,
  sessionId: string,
  fn: (s: Session) => Session,
): AppState {
  return {
    ...state,
    projects: state.projects.map((p) => ({
      ...p,
      sessions: p.sessions.map((s) => (s.id === sessionId ? fn(s) : s)),
    })),
  };
}

// ────────────────────────────────────────────────────────────
// Reducer
// ────────────────────────────────────────────────────────────
function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'SET_PHASE':
      return { ...state, phase: action.phase };

    case 'ADD_PROJECT': {
      const project = action.project;
      const firstSession = project.sessions[0];
      return {
        ...state,
        projects: [...state.projects, project],
        activeProjectId: project.id,
        activeSessionId: firstSession?.id ?? null,
      };
    }

    case 'ADD_SESSION': {
      const nextState = mapProject(state, action.projectId, (p) => ({
        ...p,
        sessions: [...p.sessions, action.session],
      }));
      return {
        ...nextState,
        activeProjectId: action.projectId,
        activeSessionId: action.session.id,
      };
    }

    case 'SET_ACTIVE':
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
      };

    case 'TOGGLE_EXPAND':
      return mapProject(state, action.projectId, (p) => ({
        ...p,
        isExpanded: !p.isExpanded,
      }));

    case 'RENAME_PROJECT':
      return mapProject(state, action.projectId, (p) => ({
        ...p,
        name: action.name,
      }));

    case 'DELETE_PROJECT': {
      const projects = state.projects.filter((p) => p.id !== action.projectId);
      let { activeProjectId, activeSessionId } = state;
      if (activeProjectId === action.projectId) {
        activeProjectId = projects[0]?.id ?? null;
        activeSessionId = projects[0]?.sessions[0]?.id ?? null;
      }
      return { ...state, projects, activeProjectId, activeSessionId };
    }

    case 'RENAME_SESSION':
      return mapProject(state, action.projectId, (p) => ({
        ...p,
        sessions: p.sessions.map((s) =>
          s.id === action.sessionId ? { ...s, title: action.title } : s,
        ),
      }));

    case 'DELETE_SESSION': {
      const nextState = mapProject(state, action.projectId, (p) => ({
        ...p,
        sessions: p.sessions.filter((s) => s.id !== action.sessionId),
      }));
      let { activeSessionId, activeProjectId } = nextState;
      if (state.activeSessionId === action.sessionId) {
        const proj = nextState.projects.find((p) => p.id === action.projectId);
        activeSessionId = proj?.sessions[0]?.id ?? null;
        activeProjectId = activeSessionId ? action.projectId : activeProjectId;
      }
      return { ...nextState, activeSessionId, activeProjectId };
    }

    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (s) => {
        // Auto-title from first user message
        let title = s.title;
        if (
          action.message.kind === 'user' &&
          s.messages.filter((m) => m.kind === 'user').length === 0
        ) {
          title = (action.message as { text: string }).text.slice(0, 20) || '新会话';
        }
        return { ...s, title, messages: [...s.messages, action.message] };
      });

    case 'APPEND_DELTA':
      return mapSession(state, action.sessionId, (s) => {
        const msgs = s.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.kind === 'assistant' && last.streaming) {
          return {
            ...s,
            messages: [
              ...msgs.slice(0, -1),
              { ...last, text: last.text + action.delta },
            ],
          };
        }
        // Create a new streaming assistant message
        const newMsg = {
          id: uuid(),
          kind: 'assistant' as const,
          text: action.delta,
          streaming: true,
          timestamp: Date.now(),
        };
        return { ...s, messages: [...msgs, newMsg] };
      });

    case 'END_STREAMING':
      return mapSession(state, action.sessionId, (s) => {
        const msgs = s.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.kind === 'assistant' && last.streaming) {
          return {
            ...s,
            messages: [...msgs.slice(0, -1), { ...last, streaming: false }],
          };
        }
        return s;
      });

    case 'UPDATE_TOOL_CALL':
      return mapSession(state, action.sessionId, (s) => ({
        ...s,
        messages: s.messages.map((m) => {
          if (m.kind === 'toolCall' && m.toolCallId === action.toolCallId) {
            return {
              ...m,
              status: action.status,
              output: action.output,
              error: action.error,
              durationMs: action.durationMs,
            };
          }
          return m;
        }),
      }));

    case 'SET_WORKING_DIR':
      return mapProject(state, action.projectId, (p) => ({
        ...p,
        workingDir: action.workingDir,
      }));

    default:
      return state;
  }
}

// ────────────────────────────────────────────────────────────
// Initial state
// ────────────────────────────────────────────────────────────
function makeInitialState(): AppState {
  const projectId = uuid();
  const sessionId = uuid();
  return {
    projects: [
      {
        id: projectId,
        name: '默认项目',
        workingDir: '',
        isExpanded: true,
        sessions: [
          {
            id: sessionId,
            projectId,
            title: '新会话',
            createdAt: Date.now(),
            messages: [],
          },
        ],
      },
    ],
    activeProjectId: projectId,
    activeSessionId: sessionId,
    phase: 'idle',
  };
}

// ────────────────────────────────────────────────────────────
// App
// ────────────────────────────────────────────────────────────
export default function App() {
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const projects = useProjects(dispatch);
  const activeSessionIdRef = useRef<string | null>(state.activeSessionId);
  const phaseRef = useRef(state.phase);

  useEffect(() => {
    activeSessionIdRef.current = state.activeSessionId;
  }, [state.activeSessionId]);

  useEffect(() => {
    phaseRef.current = state.phase;
  }, [state.phase]);

  const handleAgentEvent = useCallback((event: AgentEventPayload) => {
    const sid = activeSessionIdRef.current;

    switch (event.event) {
      case 'sessionStarted':
        // Reserved for future session management features.
        break;

      case 'phaseChanged':
        dispatch({ type: 'SET_PHASE', phase: event.data.phase });
        break;

      case 'modelDelta':
        if (!sid) {
          break;
        }
        dispatch({ type: 'APPEND_DELTA', sessionId: sid, delta: event.data.delta });
        break;

      case 'toolCallStart':
        if (!sid) {
          break;
        }
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId: sid,
          message: {
            id: uuid(),
            kind: 'toolCall',
            toolCallId: event.data.toolCallId,
            toolName: event.data.toolName,
            status: 'running',
            args: event.data.args,
            timestamp: Date.now(),
          },
        });
        break;

      case 'toolCallResult':
        if (!sid) {
          break;
        }
        dispatch({
          type: 'UPDATE_TOOL_CALL',
          sessionId: sid,
          toolCallId: event.data.result.toolCallId,
          status: event.data.result.ok ? 'ok' : 'fail',
          output: event.data.result.output,
          error: event.data.result.error,
          durationMs: event.data.result.durationMs,
        });
        break;

      case 'turnDone':
        if (sid) {
          dispatch({ type: 'END_STREAMING', sessionId: sid });
        }
        // Primary source of idle is PhaseChanged(idle). This is fallback only.
        queueMicrotask(() => {
          if (phaseRef.current !== 'idle') {
            dispatch({ type: 'SET_PHASE', phase: 'idle' });
          }
        });
        break;

      case 'error':
        if (sid && event.data.code !== 'interrupted') {
          dispatch({
            type: 'ADD_MESSAGE',
            sessionId: sid,
            message: {
              id: uuid(),
              kind: 'assistant',
              text: `错误：${event.data.message}`,
              streaming: false,
              timestamp: Date.now(),
            },
          });
        }
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
        break;
    }
  }, []);

  const { submitPrompt, interrupt, getWorkingDir } = useAgent(handleAgentEvent);

  useEffect(() => {
    const defaultProject = state.projects[0];
    if (!defaultProject || defaultProject.workingDir) {
      return;
    }

    let cancelled = false;
    void getWorkingDir()
      .then((workingDir) => {
        if (cancelled) {
          return;
        }
        dispatch({
          type: 'SET_WORKING_DIR',
          projectId: defaultProject.id,
          workingDir,
        });
        dispatch({
          type: 'RENAME_PROJECT',
          projectId: defaultProject.id,
          name: getDirectoryName(workingDir),
        });
      })
      .catch(() => {
        // Keep the placeholder project if the working directory cannot be resolved.
      });

    return () => {
      cancelled = true;
    };
  }, [state.projects, getWorkingDir]);

  const activeProject =
    state.projects.find((p) => p.id === state.activeProjectId) ?? null;
  const activeSession =
    activeProject?.sessions.find((s) => s.id === state.activeSessionId) ?? null;

  const handleNewSession = () => {
    if (state.activeProjectId) {
      projects.addSession(state.activeProjectId);
    }
  };

  const handleSetActive = (projectId: string, sessionId: string) =>
    dispatch({ type: 'SET_ACTIVE', projectId, sessionId });

  const handleToggleExpand = (projectId: string) =>
    dispatch({ type: 'TOGGLE_EXPAND', projectId });

  const handleRenameProject = (projectId: string, name: string) =>
    dispatch({ type: 'RENAME_PROJECT', projectId, name });

  const handleDeleteProject = (projectId: string) =>
    dispatch({ type: 'DELETE_PROJECT', projectId });

  const handleRenameSession = (
    projectId: string,
    sessionId: string,
    title: string,
  ) => dispatch({ type: 'RENAME_SESSION', projectId, sessionId, title });

  const handleDeleteSession = (projectId: string, sessionId: string) =>
    dispatch({ type: 'DELETE_SESSION', projectId, sessionId });

  const handleSubmit = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) {
        return;
      }

      const sid = activeSessionIdRef.current;
      if (!sid) {
        return;
      }

      dispatch({
        type: 'ADD_MESSAGE',
        sessionId: sid,
        message: {
          id: uuid(),
          kind: 'user',
          text: trimmed,
          timestamp: Date.now(),
        },
      });

      try {
        await submitPrompt(trimmed);
      } catch (err) {
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId: sid,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: `错误：${String(err)}`,
            streaming: false,
            timestamp: Date.now(),
          },
        });
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
      }
    },
    [submitPrompt],
  );

  return (
    <div
      style={{
        display: 'flex',
        height: '100vh',
        overflow: 'hidden',
        background: '#1a1a1a',
      }}
    >
      <Sidebar
        projects={state.projects}
        activeSessionId={state.activeSessionId}
        phase={state.phase}
        onSetActive={handleSetActive}
        onToggleExpand={handleToggleExpand}
        onNewProject={projects.addProject}
        onRenameProject={handleRenameProject}
        onDeleteProject={handleDeleteProject}
        onRenameSession={handleRenameSession}
        onDeleteSession={handleDeleteSession}
      />
      <Chat
        project={activeProject}
        session={activeSession}
        phase={state.phase}
        onNewSession={handleNewSession}
        onSubmitPrompt={handleSubmit}
        onInterrupt={interrupt}
      />
    </div>
  );
}
