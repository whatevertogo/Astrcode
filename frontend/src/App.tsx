import React, {
  startTransition,
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
} from 'react';
import type {
  AgentEventPayload,
  Action,
  AppState,
  Project,
  Session,
  SessionMeta,
  Message,
} from './types';
import { uuid } from './utils/uuid';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import SettingsModal from './components/Settings/SettingsModal';
import { useAgent, SessionMessage } from './hooks/useAgent';
import { releaseTurnMapping } from './lib/turnRouting';

function getDirectoryName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || '默认项目';
}

function toEpochMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function convertSessionMessage(message: SessionMessage): Message {
  const timestamp =
    message.kind === 'user' || message.kind === 'assistant'
      ? toEpochMs(message.timestamp)
      : Date.now();
  const base = { id: uuid(), timestamp };

  switch (message.kind) {
    case 'user':
      return { ...base, kind: 'user' as const, text: message.content };
    case 'assistant':
      return {
        ...base,
        kind: 'assistant' as const,
        text: message.content,
        streaming: false,
      };
    case 'toolCall':
      return {
        ...base,
        kind: 'toolCall' as const,
        toolCallId: message.toolCallId,
        toolName: message.toolName,
        status: message.success ? ('ok' as const) : ('fail' as const),
        args: message.args,
        output: message.output,
        durationMs: message.durationMs,
      };
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

function mapProject(state: AppState, projectId: string, fn: (project: Project) => Project): AppState {
  return {
    ...state,
    projects: state.projects.map((project) => (project.id === projectId ? fn(project) : project)),
  };
}

function mapSession(state: AppState, sessionId: string, fn: (session: Session) => Session): AppState {
  return {
    ...state,
    projects: state.projects.map((project) => ({
      ...project,
      sessions: project.sessions.map((session) => (session.id === sessionId ? fn(session) : session)),
    })),
  };
}

function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'SET_PHASE':
      if (state.phase === action.phase) {
        return state;
      }
      return { ...state, phase: action.phase };

    case 'SET_ACTIVE':
      return {
        ...state,
        activeProjectId: action.projectId,
        activeSessionId: action.sessionId,
      };

    case 'TOGGLE_EXPAND':
      return mapProject(state, action.projectId, (project) => ({
        ...project,
        isExpanded: !project.isExpanded,
      }));

    case 'DELETE_PROJECT': {
      const projects = state.projects.filter((project) => project.id !== action.projectId);
      let activeProjectId = state.activeProjectId;
      let activeSessionId = state.activeSessionId;
      if (activeProjectId === action.projectId) {
        activeProjectId = projects[0]?.id ?? null;
        activeSessionId = projects[0]?.sessions[0]?.id ?? null;
      }
      return { ...state, projects, activeProjectId, activeSessionId };
    }

    case 'DELETE_SESSION': {
      const nextState = mapProject(state, action.projectId, (project) => ({
        ...project,
        sessions: project.sessions.filter((session) => session.id !== action.sessionId),
      }));
      let activeSessionId = nextState.activeSessionId;
      let activeProjectId = nextState.activeProjectId;
      if (state.activeSessionId === action.sessionId) {
        const project = nextState.projects.find((item) => item.id === action.projectId);
        activeSessionId = project?.sessions[0]?.id ?? null;
        activeProjectId = project?.id ?? nextState.projects[0]?.id ?? null;
      }
      return { ...nextState, activeProjectId, activeSessionId };
    }

    case 'ADD_MESSAGE':
      return mapSession(state, action.sessionId, (session) => {
        let title = session.title;
        if (
          action.message.kind === 'user' &&
          session.messages.filter((message) => message.kind === 'user').length === 0
        ) {
          title = action.message.text.slice(0, 20) || '新会话';
        }
        return { ...session, title, messages: [...session.messages, action.message] };
      });

    case 'APPEND_DELTA':
      return mapSession(state, action.sessionId, (session) => {
        const messages = session.messages;
        const lastMessage = messages[messages.length - 1];
        if (lastMessage && lastMessage.kind === 'assistant' && lastMessage.streaming) {
          return {
            ...session,
            messages: [
              ...messages.slice(0, -1),
              { ...lastMessage, text: lastMessage.text + action.delta },
            ],
          };
        }
        return {
          ...session,
          messages: [
            ...messages,
            {
              id: uuid(),
              kind: 'assistant' as const,
              text: action.delta,
              streaming: true,
              timestamp: Date.now(),
            },
          ],
        };
      });

    case 'FINALIZE_ASSISTANT':
      return mapSession(state, action.sessionId, (session) => {
        const messages = session.messages;
        const lastMessage = messages[messages.length - 1];
        if (lastMessage && lastMessage.kind === 'assistant' && lastMessage.streaming) {
          return {
            ...session,
            messages: [
              ...messages.slice(0, -1),
              {
                ...lastMessage,
                text: action.content,
                streaming: false,
              },
            ],
          };
        }
        return {
          ...session,
          messages: [
            ...messages,
            {
              id: uuid(),
              kind: 'assistant',
              text: action.content,
              streaming: false,
              timestamp: Date.now(),
            },
          ],
        };
      });

    case 'END_STREAMING':
      return mapSession(state, action.sessionId, (session) => {
        const messages = session.messages;
        const lastMessage = messages[messages.length - 1];
        if (lastMessage && lastMessage.kind === 'assistant' && lastMessage.streaming) {
          return {
            ...session,
            messages: [...messages.slice(0, -1), { ...lastMessage, streaming: false }],
          };
        }
        return session;
      });

    case 'UPDATE_TOOL_CALL':
      return mapSession(state, action.sessionId, (session) => ({
        ...session,
        messages: session.messages.map((message) => {
          if (message.kind === 'toolCall' && message.toolCallId === action.toolCallId) {
            return {
              ...message,
              status: action.status,
              output: action.output,
              error: action.error,
              durationMs: action.durationMs,
            };
          }
          return message;
        }),
      }));

    case 'INITIALIZE':
      return {
        ...state,
        projects: action.projects,
        activeProjectId: action.activeProjectId,
        activeSessionId: action.activeSessionId,
      };

    case 'REPLACE_SESSION_MESSAGES':
      return mapSession(state, action.sessionId, (session) => ({
        ...session,
        messages: action.messages,
      }));

    default:
      return state;
  }
}

function makeInitialState(): AppState {
  return {
    projects: [],
    activeProjectId: null,
    activeSessionId: null,
    phase: 'idle',
  };
}

export default function App() {
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const [showSettings, setShowSettings] = useState(false);
  const [modelRefreshKey, setModelRefreshKey] = useState(0);
  const activeSessionIdRef = useRef<string | null>(state.activeSessionId);
  const phaseRef = useRef(state.phase);
  const turnSessionMapRef = useRef<Record<string, string>>({});

  useEffect(() => {
    activeSessionIdRef.current = state.activeSessionId;
  }, [state.activeSessionId]);

  useEffect(() => {
    phaseRef.current = state.phase;
  }, [state.phase]);

  const handleAgentEvent = useCallback((event: AgentEventPayload) => {
    const resolveSessionId = (turnId?: string | null): string | null => {
      if (turnId && turnSessionMapRef.current[turnId]) {
        return turnSessionMapRef.current[turnId];
      }
      return activeSessionIdRef.current;
    };

    switch (event.event) {
      case 'sessionStarted':
        break;

      case 'phaseChanged': {
        if (event.data.turnId) {
          resolveSessionId(event.data.turnId);
        }
        dispatch({ type: 'SET_PHASE', phase: event.data.phase });
        break;
      }

      case 'modelDelta': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        startTransition(() => {
          dispatch({ type: 'APPEND_DELTA', sessionId, delta: event.data.delta });
        });
        break;
      }

      case 'assistantMessage': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        dispatch({
          type: 'FINALIZE_ASSISTANT',
          sessionId,
          content: event.data.content,
        });
        break;
      }

      case 'toolCallStart': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
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
      }

      case 'toolCallResult': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        dispatch({
          type: 'UPDATE_TOOL_CALL',
          sessionId,
          toolCallId: event.data.result.toolCallId,
          status: event.data.result.ok ? 'ok' : 'fail',
          output: event.data.result.output,
          error: event.data.result.error,
          durationMs: event.data.result.durationMs,
        });
        break;
      }

      case 'turnDone': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (sessionId) {
          dispatch({ type: 'END_STREAMING', sessionId });
        }
        releaseTurnMapping(turnSessionMapRef.current, event.data.turnId);
        queueMicrotask(() => {
          if (phaseRef.current !== 'idle') {
            dispatch({ type: 'SET_PHASE', phase: 'idle' });
          }
        });
        break;
      }

      case 'error': {
        const sessionId = resolveSessionId(event.data.turnId ?? null);
        if (sessionId && event.data.code !== 'interrupted') {
          dispatch({
            type: 'ADD_MESSAGE',
            sessionId,
            message: {
              id: uuid(),
              kind: 'assistant',
              text: `错误：${event.data.message}`,
              streaming: false,
              timestamp: Date.now(),
            },
          });
        }
        if (event.data.turnId) {
          releaseTurnMapping(turnSessionMapRef.current, event.data.turnId);
        }
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
        break;
      }
    }
  }, []);

  const {
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
    hostBridge,
  } = useAgent(handleAgentEvent);

  const loadAndActivateSession = useCallback(
    async (projectId: string, sessionId: string) => {
      disconnectSession();
      dispatch({ type: 'SET_ACTIVE', projectId, sessionId });
      const snapshot = await loadSession(sessionId);
      dispatch({
        type: 'REPLACE_SESSION_MESSAGES',
        sessionId,
        messages: snapshot.messages.map(convertSessionMessage),
      });
      await connectSession(sessionId, snapshot.cursor);
      setModelRefreshKey((value) => value + 1);
    },
    [connectSession, disconnectSession, loadSession],
  );

  const refreshSessions = useCallback(
    async (preferredSessionId?: string | null) => {
      const sessionMetas = await listSessionsWithMeta();
      const projects = groupSessionsByProject(sessionMetas);
      const availableSessionIds = new Set(sessionMetas.map((meta) => meta.sessionId));
      const nextSessionId =
        preferredSessionId && availableSessionIds.has(preferredSessionId)
          ? preferredSessionId
          : state.activeSessionId && availableSessionIds.has(state.activeSessionId)
            ? state.activeSessionId
            : projects[0]?.sessions[0]?.id ?? null;
      const nextProjectId =
        projects.find((project) => project.sessions.some((session) => session.id === nextSessionId))
          ?.id ?? null;

      dispatch({
        type: 'INITIALIZE',
        projects,
        activeProjectId: nextProjectId,
        activeSessionId: nextSessionId,
      });

      if (nextProjectId && nextSessionId) {
        await loadAndActivateSession(nextProjectId, nextSessionId);
      } else {
        disconnectSession();
      }
    },
    [disconnectSession, listSessionsWithMeta, loadAndActivateSession, state.activeSessionId],
  );

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        await refreshSessions();
      } catch (error) {
        if (!cancelled) {
          console.error('Failed to initialize sessions:', error);
        }
      }
    })();

    return () => {
      cancelled = true;
      disconnectSession();
    };
  }, [disconnectSession, refreshSessions]);

  const activeProject = state.projects.find((project) => project.id === state.activeProjectId) ?? null;
  const activeSession = activeProject?.sessions.find((session) => session.id === state.activeSessionId) ?? null;

  const handleNewProject = async (workingDir: string) => {
    try {
      const created = await createSession(workingDir);
      await refreshSessions(created.sessionId);
    } catch (error) {
      console.error('Failed to create project session:', error);
    }
  };

  const handleNewSession = async () => {
    if (!activeProject?.workingDir) {
      return;
    }
    try {
      const created = await createSession(activeProject.workingDir);
      await refreshSessions(created.sessionId);
    } catch (error) {
      console.error('Failed to create session:', error);
    }
  };

  const handleSetActive = async (projectId: string, sessionId: string) => {
    try {
      await loadAndActivateSession(projectId, sessionId);
    } catch (error) {
      console.error('Failed to activate session:', error);
    }
  };

  const handleToggleExpand = (projectId: string) => {
    dispatch({ type: 'TOGGLE_EXPAND', projectId });
  };

  const handleDeleteProject = async (projectId: string) => {
    const project = state.projects.find((item) => item.id === projectId);
    if (!project) {
      return;
    }

    const confirmed = window.confirm(`删除项目“${project.name}”会移除该目录下所有会话，是否继续？`);
    if (!confirmed) {
      return;
    }

    try {
      const result = await deleteProject(project.workingDir);
      if (result.failedSessionIds.length > 0) {
        console.error('部分会话删除失败:', result.failedSessionIds);
      }
      await refreshSessions();
    } catch (error) {
      console.error('Failed to delete project:', error);
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
    } catch (error) {
      console.error('Failed to delete session:', error);
    }
  };

  const handleSubmit = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) {
        return;
      }

      const sessionId = activeSessionIdRef.current;
      if (!sessionId) {
        return;
      }

      dispatch({
        type: 'ADD_MESSAGE',
        sessionId,
        message: {
          id: uuid(),
          kind: 'user',
          text: trimmed,
          timestamp: Date.now(),
        },
      });

      try {
        const turnId = await submitPrompt(sessionId, trimmed);
        turnSessionMapRef.current[turnId] = sessionId;
      } catch (error) {
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: `错误：${String(error)}`,
            streaming: false,
            timestamp: Date.now(),
          },
        });
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
      }
    },
    [submitPrompt],
  );

  const handleInterrupt = useCallback(async () => {
    if (!activeSessionIdRef.current) {
      return;
    }
    await interrupt(activeSessionIdRef.current);
  }, [interrupt]);

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
        canSelectDirectory={hostBridge.canSelectDirectory}
        defaultWorkingDir={activeProject?.workingDir}
        onSelectDirectory={selectDirectory}
        onSetActive={handleSetActive}
        onToggleExpand={handleToggleExpand}
        onNewProject={handleNewProject}
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
        onInterrupt={handleInterrupt}
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
