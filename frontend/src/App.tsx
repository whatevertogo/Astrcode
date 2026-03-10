import React, { startTransition, useCallback, useEffect, useReducer, useRef, useState } from 'react';
import type {
  AgentEventPayload,
  Action,
  AppState,
  SessionMeta,
  Project,
  Session,
  Message,
} from './types';
import { uuid } from './utils/uuid';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import SettingsModal from './components/Settings/SettingsModal';
import { useAgent, SessionMessage } from './hooks/useAgent';
import { useProjects } from './hooks/useProjects';

function getDirectoryName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || '默认项目';
}

function toEpochMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function convertSessionMessage(m: SessionMessage): Message {
  const timestamp = (m.kind === 'user' || m.kind === 'assistant')
    ? toEpochMs(m.timestamp)
    : Date.now();
  const base = { id: uuid(), timestamp };
  switch (m.kind) {
    case 'user':
      return { ...base, kind: 'user' as const, text: m.content };
    case 'assistant':
      return { ...base, kind: 'assistant' as const, text: m.content, streaming: false };
    case 'toolCall':
      return {
        ...base,
        kind: 'toolCall' as const,
        toolCallId: m.toolCallId,
        toolName: m.toolName,
        status: m.success ? 'ok' as const : 'fail' as const,
        args: m.args,
        output: m.output,
        durationMs: m.durationMs,
      };
    default:
      return { ...base, kind: 'assistant' as const, text: '', streaming: false };
  }
}

function groupSessionsByProject(sessionMetas: SessionMeta[]): Project[] {
  const projectMap = new Map<string, { project: Project; maxUpdatedAt: number }>();

  for (const meta of sessionMetas) {
    const projectId = meta.workingDir || '__default_project__';
    const projectName = meta.displayName || getDirectoryName(meta.workingDir);
    const updatedAt = toEpochMs(meta.updatedAt);
    const createdAt = toEpochMs(meta.createdAt);

    let holder = projectMap.get(projectId);
    if (!holder) {
      holder = {
        project: {
          id: projectId,
          name: projectName,
          workingDir: meta.workingDir,
          isExpanded: true,
          sessions: [],
        },
        maxUpdatedAt: updatedAt,
      };
      projectMap.set(projectId, holder);
    } else {
      holder.maxUpdatedAt = Math.max(holder.maxUpdatedAt, updatedAt);
    }

    holder.project.sessions.push({
      id: meta.sessionId,
      projectId,
      title: meta.title || '新会话',
      createdAt,
      updatedAt,
      messages: [],
    });
  }

  const projects = Array.from(projectMap.values());
  projects.sort((a, b) => b.maxUpdatedAt - a.maxUpdatedAt);
  return projects.map((item) => {
    item.project.sessions.sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
    return item.project;
  });
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
      if (state.phase === action.phase) {
        return state;
      }
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

    case 'INITIALIZE': {
      return {
        ...state,
        projects: action.projects,
        activeProjectId: action.activeProjectId,
        activeSessionId: action.activeSessionId,
      };
    }

    case 'REPLACE_SESSION_MESSAGES':
      return mapSession(state, action.sessionId, (s) => ({
        ...s,
        messages: action.messages,
      }));

    case 'ADD_SESSION_BACKEND': {
      // Add a new session from backend (after newSession call)
      const newSession: Session = {
        id: action.sessionId,
        projectId: action.projectId,
        title: '新会话',
        createdAt: Date.now(),
        messages: [],
      };
      const nextState = mapProject(state, action.projectId, (p) => ({
        ...p,
        sessions: [newSession, ...p.sessions],
      }));
      return {
        ...nextState,
        activeSessionId: action.sessionId,
      };
    }

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
  const [showSettings, setShowSettings] = useState(false);
  const [modelRefreshKey, setModelRefreshKey] = useState(0);
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
        startTransition(() => {
          dispatch({ type: 'APPEND_DELTA', sessionId: sid!, delta: event.data.delta });
        });
        break;

      case 'assistantMessage':
        if (!sid) {
          break;
        }
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId: sid,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: event.data.content,
            streaming: false,
            timestamp: Date.now(),
          },
        });
        break;

      case 'toolCallStart':
        if (!sid) {
          break;
        }
        {
          const data = event.data as typeof event.data & {
            tool_call_id?: string;
            tool_name?: string;
            turn_id?: string;
          };
          dispatch({
            type: 'ADD_MESSAGE',
            sessionId: sid,
            message: {
              id: uuid(),
              kind: 'toolCall',
              toolCallId: data.toolCallId ?? data.tool_call_id ?? 'unknown',
              toolName: data.toolName ?? data.tool_name ?? '(unknown tool)',
              status: 'running',
              args: event.data.args,
              timestamp: Date.now(),
            },
          });
        }
        break;

      case 'toolCallResult':
        if (!sid) {
          break;
        }
        {
          const result = event.data.result as typeof event.data.result & {
            tool_call_id?: string;
            duration_ms?: number;
          };
          dispatch({
            type: 'UPDATE_TOOL_CALL',
            sessionId: sid,
            toolCallId: result.toolCallId ?? result.tool_call_id ?? 'unknown',
            status: event.data.result.ok ? 'ok' : 'fail',
            output: event.data.result.output,
            error: event.data.result.error,
            durationMs: result.durationMs ?? result.duration_ms ?? 0,
          });
        }
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

  const {
    submitPrompt,
    interrupt,
    getWorkingDir,
    getSessionId,
    listSessionsWithMeta,
    loadSession,
    switchSession,
    newSession,
    deleteSession,
    deleteProject,
    getConfig,
    saveActiveSelection,
    setModel,
    getCurrentModel,
    listAvailableModels,
    testConnection,
    openConfigInEditor,
  } = useAgent(handleAgentEvent);

  const refreshSessions = useCallback(async () => {
    const sessionMetas = await listSessionsWithMeta();
    let metas = sessionMetas;

    if (metas.length === 0) {
      await newSession();
      metas = await listSessionsWithMeta();
    }

    const projectsFromMeta = groupSessionsByProject(metas);
    const availableSessionIds = new Set(metas.map((meta) => meta.sessionId));
    const backendCurrentSessionId = await getSessionId();
    const fallbackSessionId = metas[0]?.sessionId ?? '';
    const resolvedCurrentSessionId = availableSessionIds.has(backendCurrentSessionId)
      ? backendCurrentSessionId
      : fallbackSessionId;

    const messages = resolvedCurrentSessionId
      ? await loadSession(resolvedCurrentSessionId)
      : [];
    const convertedMessages = messages.map(convertSessionMessage);

    const projects = projectsFromMeta.map((project) => ({
      ...project,
      sessions: project.sessions.map((session) => (
        session.id === resolvedCurrentSessionId
          ? { ...session, messages: convertedMessages }
          : session
      )),
    }));

    let activeProjectId = projects.find((project) =>
      project.sessions.some((session) => session.id === resolvedCurrentSessionId),
    )?.id ?? null;
    let activeSessionId: string | null = resolvedCurrentSessionId || null;

    if (!activeProjectId && projects.length > 0) {
      activeProjectId = projects[0].id;
      activeSessionId = projects[0].sessions[0]?.id ?? null;
    }

    if (projects.length === 0) {
      const workingDir = await getWorkingDir();
      const projectId = '__default_project__';
      const fallbackId = activeSessionId ?? `web-${Date.now()}`;
      dispatch({
        type: 'INITIALIZE',
        projects: [{
          id: projectId,
          name: getDirectoryName(workingDir),
          workingDir,
          isExpanded: true,
          sessions: [{
            id: fallbackId,
            projectId,
            title: '新会话',
            createdAt: Date.now(),
            updatedAt: Date.now(),
            messages: convertedMessages,
          }],
        }],
        activeProjectId: projectId,
        activeSessionId: fallbackId,
      });
      return;
    }

    dispatch({
      type: 'INITIALIZE',
      projects,
      activeProjectId,
      activeSessionId,
    });
  }, [getSessionId, getWorkingDir, listSessionsWithMeta, loadSession, newSession]);

  // Initialize session data from backend on mount
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        await refreshSessions();
      } catch (err) {
        if (!cancelled) {
          console.error('Failed to initialize sessions:', err);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [refreshSessions]);

  const activeProject =
    state.projects.find((p) => p.id === state.activeProjectId) ?? null;
  const activeSession =
    activeProject?.sessions.find((s) => s.id === state.activeSessionId) ?? null;

  const handleNewSession = async () => {
    try {
      await newSession();
      await refreshSessions();
    } catch (err) {
      console.error('Failed to create new session:', err);
    }
  };

  const handleSetActive = async (projectId: string, sessionId: string) => {
    // If switching to a different session, call backend
    if (sessionId !== state.activeSessionId) {
      try {
        await switchSession(sessionId);
        setModelRefreshKey((value) => value + 1);
        // Load session messages if not already loaded
        const targetProject = state.projects.find((p) => p.id === projectId);
        const targetSession = targetProject?.sessions.find((s) => s.id === sessionId);
        if (targetSession && targetSession.messages.length === 0) {
          const messages = await loadSession(sessionId);
          // Convert SessionMessage to Message
          const convertedMessages: Message[] = messages.map((m) => {
            const base = { id: uuid(), timestamp: Date.now() };
            switch (m.kind) {
              case 'user':
                return { ...base, kind: 'user' as const, text: m.content };
              case 'assistant':
                return { ...base, kind: 'assistant' as const, text: m.content, streaming: false };
              case 'toolCall':
                return {
                  ...base,
                  kind: 'toolCall' as const,
                  toolCallId: m.toolCallId,
                  toolName: m.toolName,
                  status: m.success ? 'ok' as const : 'fail' as const,
                  args: m.args,
                  output: m.output,
                  durationMs: m.durationMs,
                };
              default:
                return { ...base, kind: 'assistant' as const, text: '', streaming: false };
            }
          });
          dispatch({
            type: 'REPLACE_SESSION_MESSAGES',
            sessionId,
            messages: convertedMessages,
          });
        }
      } catch (err) {
        console.error('Failed to switch session:', err);
        return;
      }
    }
    dispatch({ type: 'SET_ACTIVE', projectId, sessionId });
  };

  const handleToggleExpand = (projectId: string) =>
    dispatch({ type: 'TOGGLE_EXPAND', projectId });

  const handleDeleteProject = async (projectId: string) => {
    const project = state.projects.find((p) => p.id === projectId);
    if (!project) {
      return;
    }
    const confirmed = window.confirm(
      `删除项目“${project.name}”会移除该目录下所有会话，是否继续？`,
    );
    if (!confirmed) {
      return;
    }
    try {
      const result = await deleteProject(project.workingDir);
      if (result.failedSessionIds.length > 0) {
        console.error('部分会话删除失败:', result.failedSessionIds);
      }
      await refreshSessions();
    } catch (err) {
      console.error('Failed to delete project:', err);
    }
  };

  const handleDeleteSession = async (_projectId: string, sessionId: string) => {
    const confirmed = window.confirm('确认删除该会话？该操作不可恢复。');
    if (!confirmed) {
      return;
    }
    try {
      await deleteSession(sessionId);
      await refreshSessions();
    } catch (err) {
      console.error('Failed to delete session:', err);
    }
  };

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
        await submitPrompt(trimmed, activeSession?.messages ?? []);
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
    [activeSession, submitPrompt],
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
        onDeleteProject={handleDeleteProject}
        onDeleteSession={handleDeleteSession}
        onOpenSettings={() => setShowSettings(true)}
      />
      <Chat
        project={activeProject}
        session={activeSession}
        phase={state.phase}
        onNewSession={handleNewSession}
        onSubmitPrompt={handleSubmit}
        onInterrupt={interrupt}
        modelRefreshKey={modelRefreshKey}
        getCurrentModel={getCurrentModel}
        listAvailableModels={listAvailableModels}
        setModel={setModel}
      />
      {showSettings && (
        <SettingsModal
          onClose={() => setShowSettings(false)}
          getConfig={getConfig}
          saveActiveSelection={saveActiveSelection}
          testConnection={testConnection}
          openConfigInEditor={openConfigInEditor}
        />
      )}
    </div>
  );
}
