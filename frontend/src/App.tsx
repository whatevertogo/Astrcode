import {
  startTransition,
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import type {
  AgentEventPayload,
  Action,
  AppState,
  Session,
  Message,
  Project,
  SessionMeta,
} from './types';
import { uuid } from './utils/uuid';
import { reducer as appReducer, makeInitialState } from './store/reducer';
import {
  convertSessionMessage,
  groupSessionsByProject,
  replaceSessionMessages,
} from './store/utils';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import SettingsModal from './components/Settings/SettingsModal';
import { useAgent, SessionMessage } from './hooks/useAgent';
import { releaseTurnMapping, resolveSessionForTurn } from './lib/turnRouting';
import styles from './App.module.css';

const reducer = appReducer;



const DEFAULT_SIDEBAR_WIDTH = 260;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 420;

const getMaxSidebarWidth = (): number =>
  Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, window.innerWidth - 360));

const clampSidebarWidth = (width: number): number =>
  Math.min(getMaxSidebarWidth(), Math.max(MIN_SIDEBAR_WIDTH, width));

export default function App() {
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const [showSettings, setShowSettings] = useState(false);
  const [modelRefreshKey, setModelRefreshKey] = useState(0);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    if (typeof window === 'undefined') {
      return DEFAULT_SIDEBAR_WIDTH;
    }
    const savedWidth = Number(window.localStorage.getItem('astrcode.sidebarWidth'));
    return Number.isFinite(savedWidth) ? clampSidebarWidth(savedWidth) : DEFAULT_SIDEBAR_WIDTH;
  });
  const [isResizingSidebar, setIsResizingSidebar] = useState(false);
  const activeSessionIdRef = useRef<string | null>(state.activeSessionId);
  const phaseRef = useRef(state.phase);
  const turnSessionMapRef = useRef<Record<string, string>>({});
  const pendingSubmitSessionRef = useRef<string[]>([]);
  const sessionActivationGenerationRef = useRef(0);
  const sidebarDragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const releasePendingSubmitSession = useCallback((sessionId: string) => {
    const queue = pendingSubmitSessionRef.current;
    const index = queue.indexOf(sessionId);
    if (index >= 0) {
      queue.splice(index, 1);
    }
  }, []);

  useEffect(() => {
    activeSessionIdRef.current = state.activeSessionId;
  }, [state.activeSessionId]);

  useEffect(() => {
    phaseRef.current = state.phase;
  }, [state.phase]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }
    window.localStorage.setItem('astrcode.sidebarWidth', String(sidebarWidth));
  }, [sidebarWidth]);

  useEffect(() => {
    const handleResize = () => {
      setSidebarWidth((width) => clampSidebarWidth(width));
    };

    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  useEffect(() => {
    return () => {
      document.body.style.removeProperty('cursor');
      document.body.style.removeProperty('user-select');
    };
  }, []);

  const finishSidebarResize = useCallback(() => {
    sidebarDragRef.current = null;
    setIsResizingSidebar(false);
    document.body.style.removeProperty('cursor');
    document.body.style.removeProperty('user-select');
  }, []);

  useEffect(() => {
    if (!isResizingSidebar) {
      return;
    }

    const handlePointerMove = (event: PointerEvent) => {
      const dragState = sidebarDragRef.current;
      if (!dragState) {
        return;
      }

      setSidebarWidth(clampSidebarWidth(dragState.startWidth + event.clientX - dragState.startX));
    };

    const handlePointerUp = () => {
      finishSidebarResize();
    };

    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', handlePointerUp);
    window.addEventListener('pointercancel', handlePointerUp);

    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
      window.removeEventListener('pointercancel', handlePointerUp);
    };
  }, [finishSidebarResize, isResizingSidebar]);

  const handleAgentEvent = (event: AgentEventPayload) => {
    const resolveSessionId = (turnId?: string | null): string | null => {
      return resolveSessionForTurn(
        turnSessionMapRef.current,
        pendingSubmitSessionRef.current,
        turnId,
        activeSessionIdRef.current
      );
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
          dispatch({
            type: 'APPEND_DELTA',
            sessionId,
            turnId: event.data.turnId,
            delta: event.data.delta,
          });
        });
        break;
      }

      case 'thinkingDelta': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        startTransition(() => {
          dispatch({
            type: 'APPEND_REASONING_DELTA',
            sessionId,
            turnId: event.data.turnId,
            delta: event.data.delta,
          });
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
          turnId: event.data.turnId,
          content: event.data.content,
          reasoningText: event.data.reasoningContent,
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
            turnId: event.data.turnId,
            toolCallId: event.data.toolCallId,
            toolName: event.data.toolName,
            status: 'running',
            args: event.data.args,
            timestamp: Date.now(),
          },
        });
        break;
      }

      case 'toolCallDelta': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (!sessionId) {
          break;
        }
        startTransition(() => {
          dispatch({
            type: 'APPEND_TOOL_CALL_DELTA',
            sessionId,
            turnId: event.data.turnId,
            toolCallId: event.data.toolCallId,
            toolName: event.data.toolName,
            stream: event.data.stream,
            delta: event.data.delta,
          });
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
          turnId: event.data.turnId,
          toolCallId: event.data.result.toolCallId,
          toolName: event.data.result.toolName,
          status: event.data.result.ok ? 'ok' : 'fail',
          output: event.data.result.output,
          error: event.data.result.error,
          metadata: event.data.result.metadata,
          durationMs: event.data.result.durationMs,
        });
        break;
      }

      case 'turnDone': {
        const sessionId = resolveSessionId(event.data.turnId);
        if (sessionId) {
          dispatch({ type: 'END_STREAMING', sessionId, turnId: event.data.turnId });
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
              reasoningText: '',
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
  };

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
      const activationGeneration = ++sessionActivationGenerationRef.current;
      disconnectSession();
      const snapshot = await loadSession(sessionId);
      if (activationGeneration !== sessionActivationGenerationRef.current) {
        return;
      }
      dispatch({
        type: 'REPLACE_SESSION_MESSAGES',
        sessionId,
        messages: snapshot.messages.map(convertSessionMessage),
      });
      // 先写入快照，再切换 active，避免会话切换瞬间渲染空白列表。
      activeSessionIdRef.current = sessionId;
      dispatch({ type: 'SET_ACTIVE', projectId, sessionId });
      await connectSession(sessionId, snapshot.cursor);
      if (activationGeneration !== sessionActivationGenerationRef.current) {
        return;
      }
      setModelRefreshKey((value) => value + 1);
    },
    [connectSession, disconnectSession, loadSession]
  );

  const refreshSessions = useCallback(
    async (preferredSessionId?: string | null) => {
      const activationGeneration = ++sessionActivationGenerationRef.current;
      const sessionMetas = await listSessionsWithMeta();
      const projects = groupSessionsByProject(sessionMetas);
      const availableSessionIds = new Set(sessionMetas.map((meta) => meta.sessionId));
      const nextSessionId =
        preferredSessionId && availableSessionIds.has(preferredSessionId)
          ? preferredSessionId
          : activeSessionIdRef.current && availableSessionIds.has(activeSessionIdRef.current)
            ? activeSessionIdRef.current
            : (projects[0]?.sessions[0]?.id ?? null);
      const nextProjectId =
        projects.find((project) => project.sessions.some((session) => session.id === nextSessionId))
          ?.id ?? null;

      if (nextProjectId && nextSessionId) {
        disconnectSession();
        const snapshot = await loadSession(nextSessionId);
        if (activationGeneration !== sessionActivationGenerationRef.current) {
          return;
        }
        const hydratedProjects = replaceSessionMessages(
          projects,
          nextSessionId,
          snapshot.messages.map(convertSessionMessage)
        );
        activeSessionIdRef.current = nextSessionId;
        dispatch({
          type: 'INITIALIZE',
          projects: hydratedProjects,
          activeProjectId: nextProjectId,
          activeSessionId: nextSessionId,
        });
        await connectSession(nextSessionId, snapshot.cursor);
        if (activationGeneration !== sessionActivationGenerationRef.current) {
          return;
        }
        setModelRefreshKey((value) => value + 1);
        return;
      }

      activeSessionIdRef.current = null;
      dispatch({
        type: 'INITIALIZE',
        projects,
        activeProjectId: nextProjectId,
        activeSessionId: nextSessionId,
      });
      disconnectSession();
    },
    [connectSession, disconnectSession, listSessionsWithMeta, loadSession]
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

  const activeProject =
    state.projects.find((project) => project.id === state.activeProjectId) ?? null;
  const activeSession =
    activeProject?.sessions.find((session) => session.id === state.activeSessionId) ?? null;

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
      if (phaseRef.current !== 'idle') {
        return;
      }

      // 在请求真正发出前就切到 busy，封住同一事件循环内的双击重入窗口。
      phaseRef.current = 'thinking';
      dispatch({ type: 'SET_PHASE', phase: 'thinking' });

      pendingSubmitSessionRef.current.push(sessionId);

      try {
        const submitted = await submitPrompt(sessionId, trimmed);
        const effectiveSessionId = submitted.sessionId ?? sessionId;
        turnSessionMapRef.current[submitted.turnId] =
          turnSessionMapRef.current[submitted.turnId] ?? effectiveSessionId;
        releasePendingSubmitSession(sessionId);

        if (
          submitted.branchedFromSessionId &&
          submitted.branchedFromSessionId === sessionId &&
          effectiveSessionId !== sessionId
        ) {
          // 分叉成功后旧 session 的 turnDone 可能已经在切换期间到达并被忽略；
          // 先本地兜底回 idle，避免 UI 把“正在思考”状态卡死到下一次刷新。
          phaseRef.current = 'idle';
          dispatch({ type: 'SET_PHASE', phase: 'idle' });
          await refreshSessions(effectiveSessionId);
          return;
        }

        dispatch({
          type: 'ADD_MESSAGE',
          sessionId: effectiveSessionId,
          message: {
            id: uuid(),
            kind: 'user',
            turnId: submitted.turnId,
            text: trimmed,
            timestamp: Date.now(),
          },
        });
      } catch (error) {
        releasePendingSubmitSession(sessionId);
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: `错误：${String(error)}`,
            reasoningText: '',
            streaming: false,
            timestamp: Date.now(),
          },
        });
        phaseRef.current = 'idle';
        dispatch({ type: 'SET_PHASE', phase: 'idle' });
      }
    },
    [refreshSessions, releasePendingSubmitSession, submitPrompt]
  );

  const handleInterrupt = useCallback(async () => {
    if (!activeSessionIdRef.current) {
      return;
    }
    await interrupt(activeSessionIdRef.current);
  }, [interrupt]);

  const handleSidebarResizeStart = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      sidebarDragRef.current = {
        startX: event.clientX,
        startWidth: sidebarWidth,
      };
      setIsResizingSidebar(true);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
    },
    [sidebarWidth]
  );

  const handleSidebarResizeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      setSidebarWidth((width) => clampSidebarWidth(width - 16));
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      setSidebarWidth((width) => clampSidebarWidth(width + 16));
    }
  }, []);

  return (
    <div className={styles.app}>
      <div className={styles.sidebarPane} style={{ width: `${sidebarWidth}px` }}>
        <Sidebar
          projects={state.projects}
          activeSessionId={state.activeSessionId}
          phase={state.phase}
          canSelectDirectory={hostBridge.canSelectDirectory}
          defaultWorkingDir={activeProject?.workingDir}
          onSelectDirectory={selectDirectory}
          onSetActive={(projectId, sessionId) => {
            void handleSetActive(projectId, sessionId);
          }}
          onToggleExpand={handleToggleExpand}
          onNewProject={(workingDir) => {
            void handleNewProject(workingDir);
          }}
          onDeleteProject={(projectId) => {
            void handleDeleteProject(projectId);
          }}
          onDeleteSession={(projectId, sessionId) => {
            void handleDeleteSession(projectId, sessionId);
          }}
          onOpenSettings={() => setShowSettings(true)}
        />
      </div>
      <div
        className={`${styles.sidebarResizer} ${isResizingSidebar ? styles.sidebarResizerActive : ''}`}
        role="separator"
        aria-label="调整侧边栏宽度"
        aria-orientation="vertical"
        aria-valuemin={MIN_SIDEBAR_WIDTH}
        aria-valuemax={getMaxSidebarWidth()}
        aria-valuenow={sidebarWidth}
        tabIndex={0}
        onPointerDown={handleSidebarResizeStart}
        onKeyDown={handleSidebarResizeKeyDown}
      />
      <Chat
        project={activeProject}
        session={activeSession}
        phase={state.phase}
        onNewSession={() => {
          void handleNewSession();
        }}
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
