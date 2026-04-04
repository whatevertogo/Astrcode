import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
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
import ConfirmDialog from './components/ConfirmDialog';
import { useAgent } from './hooks/useAgent';
import { useAgentEventHandler } from './hooks/useAgentEventHandler';
import { useSessionCatalogEvents } from './hooks/useSessionCatalogEvents';
import { useSidebarResize } from './hooks/useSidebarResize';
import { cn } from './lib/utils';
import { parseRuntimeSlashCommand } from './lib/slashCommands';
import type { SessionCatalogEventPayload } from './types';

const reducer = appReducer;

export default function App() {
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const [showSettings, setShowSettings] = useState(false);
  const [modelRefreshKey, setModelRefreshKey] = useState(0);
  // 确认对话框状态（替代 window.confirm）
  const [confirmDialog, setConfirmDialog] = useState<{
    title: string;
    message: string;
    danger?: boolean;
    confirmLabel?: string;
    cancelLabel?: string;
    onConfirm: () => void | Promise<void>;
  } | null>(null);
  const activeSessionIdRef = useRef<string | null>(state.activeSessionId);
  const phaseRef = useRef(state.phase);
  const turnSessionMapRef = useRef<Record<string, string>>({});
  const pendingSubmitSessionRef = useRef<string[]>([]);
  const sessionActivationGenerationRef = useRef(0);
  const {
    sidebarWidth,
    isResizingSidebar,
    isSidebarOpen,
    toggleSidebar,
    minSidebarWidth,
    maxSidebarWidth,
    handleSidebarResizeStart,
    handleSidebarResizeKeyDown,
  } = useSidebarResize();

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
  const handleAgentEvent = useAgentEventHandler({
    activeSessionIdRef,
    pendingSubmitSessionRef,
    turnSessionMapRef,
    phaseRef,
    dispatch,
  });

  const {
    createSession,
    listSessionsWithMeta,
    loadSession,
    connectSession,
    disconnectSession,
    submitPrompt,
    interrupt,
    compactSession,
    deleteSession,
    deleteProject,
    listComposerOptions,
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

  const handleSessionCatalogEvent = useCallback(
    (event: SessionCatalogEventPayload) => {
      switch (event.event) {
        case 'sessionBranched':
          if (activeSessionIdRef.current === event.data.sourceSessionId) {
            void refreshSessions(event.data.sessionId);
            return;
          }
          void refreshSessions();
          return;
        case 'sessionCreated':
        case 'sessionDeleted':
        case 'projectDeleted':
          void refreshSessions();
          return;
      }
    },
    [refreshSessions]
  );

  useSessionCatalogEvents({
    onEvent: handleSessionCatalogEvent,
    onResync: () => {
      void refreshSessions();
    },
  });

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

  const handleDeleteProject = (projectId: string) => {
    const project = state.projects.find((item) => item.id === projectId);
    if (!project) {
      return;
    }

    setConfirmDialog({
      title: '删除项目',
      message: `删除项目"${project.name}"会移除该目录下所有会话，是否继续？`,
      danger: true,
      onConfirm: async () => {
        setConfirmDialog(null);
        try {
          const result = await deleteProject(project.workingDir);
          if (result.failedSessionIds.length > 0) {
            console.error('部分会话删除失败:', result.failedSessionIds);
          }
          await refreshSessions();
        } catch (error) {
          console.error('Failed to delete project:', error);
        }
      },
    });
  };

  const handleDeleteSession = (_projectId: string, sessionId: string) => {
    setConfirmDialog({
      title: '删除会话',
      message: '确认删除该会话？该操作不可恢复。',
      danger: true,
      onConfirm: async () => {
        setConfirmDialog(null);
        try {
          await deleteSession(sessionId);
          await refreshSessions();
        } catch (error) {
          console.error('Failed to delete session:', error);
        }
      },
    });
  };

  const handleSubmit = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) {
        return;
      }

      const slashCommand = parseRuntimeSlashCommand(trimmed);
      const sessionId = activeSessionIdRef.current;
      if (!sessionId) {
        if (slashCommand) {
          setConfirmDialog({
            title: '无法执行命令',
            message: '当前没有激活会话，无法执行 `/compact`。',
            confirmLabel: '知道了',
            cancelLabel: '关闭',
            onConfirm: () => {
              setConfirmDialog(null);
            },
          });
        }
        return;
      }

      const appendLocalError = (message: string) => {
        dispatch({
          type: 'ADD_MESSAGE',
          sessionId,
          message: {
            id: uuid(),
            kind: 'assistant',
            text: `错误：${message}`,
            reasoningText: '',
            streaming: false,
            timestamp: Date.now(),
          },
        });
      };

      if (slashCommand) {
        if (phaseRef.current !== 'idle') {
          // TODO: 后续可在这里接入命令排队，而不是直接拒绝。
          appendLocalError('当前会话正在运行，暂不允许手动 compact。');
          return;
        }

        if (slashCommand.kind === 'compactInvalidArgs') {
          appendLocalError('`/compact` 当前不接受参数，请直接输入 `/compact`。');
          return;
        }

        try {
          await compactSession(sessionId);
        } catch (error) {
          appendLocalError(error instanceof Error ? error.message : String(error));
        }
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
          // 先本地兜底回 idle，避免 UI 把"正在思考"状态卡死到下一次刷新。
          phaseRef.current = 'idle';
          dispatch({ type: 'SET_PHASE', phase: 'idle' });
          await refreshSessions(effectiveSessionId);
          return;
        }

        // 用户消息由 SSE 的 userMessage 事件通过 UPSERT_USER_MESSAGE 处理
        // 移除乐观写入以避免 StrictMode 双重渲染导致的重复消息问题
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
    [compactSession, refreshSessions, releasePendingSubmitSession, submitPrompt]
  );

  const handleInterrupt = useCallback(async () => {
    if (!activeSessionIdRef.current) {
      return;
    }
    await interrupt(activeSessionIdRef.current);
  }, [interrupt]);

  return (
    <div className="flex h-screen overflow-hidden bg-[var(--app-bg)] text-[var(--text-primary)]">
      {isSidebarOpen && (
        <>
          <div className="flex-none min-w-0" style={{ width: `${sidebarWidth}px` }}>
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
              onNewSession={() => {
                void handleNewSession();
              }}
            />
          </div>
          <div
            className={cn(
              'relative w-[10px] flex-none cursor-col-resize bg-transparent outline-none before:absolute before:inset-y-0 before:left-1/2 before:w-[1px] before:-translate-x-1/2 before:bg-[var(--border)] hover:before:w-[2px] hover:before:bg-[var(--border-strong)] focus-visible:before:w-[2px] focus-visible:before:bg-[var(--border-strong)] before:transition-all before:duration-150 before:ease-out',
              isResizingSidebar && 'before:w-[2px] before:bg-[var(--border-strong)]'
            )}
            role="separator"
            aria-label="调整侧边栏宽度"
            aria-orientation="vertical"
            aria-valuemin={minSidebarWidth}
            aria-valuemax={maxSidebarWidth}
            aria-valuenow={sidebarWidth}
            tabIndex={0}
            onPointerDown={handleSidebarResizeStart}
            onKeyDown={handleSidebarResizeKeyDown}
          />
        </>
      )}
      <div className="flex-1 min-w-0 relative flex flex-col">
        <Chat
          project={activeProject}
          session={activeSession}
          isSidebarOpen={isSidebarOpen}
          toggleSidebar={toggleSidebar}
          phase={state.phase}
          onSubmitPrompt={handleSubmit}
          onInterrupt={handleInterrupt}
          listComposerOptions={listComposerOptions}
          modelRefreshKey={modelRefreshKey}
          getCurrentModel={getCurrentModel}
          listAvailableModels={listAvailableModels}
          setModel={setModel}
        />
      </div>
      {showSettings && (
        <SettingsModal
          onClose={() => setShowSettings(false)}
          getConfig={getConfig}
          saveActiveSelection={saveActiveSelection}
          testConnection={testConnection}
          openConfigInEditor={openConfigInEditor}
        />
      )}
      {confirmDialog && (
        <ConfirmDialog
          title={confirmDialog.title}
          message={confirmDialog.message}
          danger={confirmDialog.danger}
          confirmLabel={confirmDialog.confirmLabel}
          cancelLabel={confirmDialog.cancelLabel}
          onConfirm={confirmDialog.onConfirm}
          onCancel={() => setConfirmDialog(null)}
        />
      )}
    </div>
  );
}
