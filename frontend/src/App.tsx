import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { uuid } from './utils/uuid';
import { reducer as appReducer, makeInitialState } from './store/reducer';
import { groupSessionsByProject, replaceSessionMessages } from './store/utils';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import SettingsModal from './components/Settings/SettingsModal';
import ConfirmDialog from './components/ConfirmDialog';
import { useAgent } from './hooks/useAgent';
import { useAgentEventHandler } from './hooks/useAgentEventHandler';
import { useSessionCatalogEvents } from './hooks/useSessionCatalogEvents';
import { useSidebarResize } from './hooks/useSidebarResize';
import { replaySessionHistory } from './lib/sessionHistory';
import { findMatchingSessionId, normalizeSessionIdForCompare } from './lib/sessionId';
import { buildSubRunThreadTree, listRootSubRunViews } from './lib/subRunView';
import {
  buildFocusedSubRunFilter,
  buildSubRunChildrenFilter,
  buildSessionViewLocationHref,
  readSessionViewLocation,
} from './lib/sessionView';
import { cn } from './lib/utils';
import { parseRuntimeSlashCommand } from './lib/slashCommands';
import type { SessionCatalogEventPayload } from './types';

const reducer = appReducer;

export default function App() {
  const initialViewLocationRef = useRef(readSessionViewLocation(window.location.href));
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const [activeSubRunChildren, setActiveSubRunChildren] = useState<{
    subRuns: ReturnType<typeof listRootSubRunViews>;
    contentFingerprint: string;
  }>({
    subRuns: [],
    contentFingerprint: '',
  });
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
  const activeSubRunPathRef = useRef(state.activeSubRunPath);
  const subRunTitleCacheRef = useRef(new Map<string, string>());
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
    activeSubRunPathRef.current = state.activeSubRunPath;
  }, [state.activeSubRunPath]);

  useEffect(() => {
    phaseRef.current = state.phase;
  }, [state.phase]);

  useEffect(() => {
    subRunTitleCacheRef.current.clear();
  }, [state.activeSessionId]);

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
    cancelSubRun,
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

  const loadSessionView = useCallback(
    async (sessionId: string, subRunPath: string[]) => {
      const filter = buildFocusedSubRunFilter(subRunPath);
      const [snapshot, childSnapshot] = await Promise.all([
        loadSession(sessionId, filter),
        filter?.subRunId
          ? loadSession(sessionId, buildSubRunChildrenFilter(filter.subRunId))
          : Promise.resolve(null),
      ]);
      const replayed = replaySessionHistory(sessionId, snapshot.events, snapshot.phase);

      if (!filter?.subRunId || !childSnapshot) {
        return {
          filter,
          cursor: snapshot.cursor,
          phase: replayed.phase,
          messages: replayed.messages,
          childSubRuns: [] as ReturnType<typeof listRootSubRunViews>,
          childContentFingerprint: '',
        };
      }

      const childReplayed = replaySessionHistory(
        sessionId,
        childSnapshot.events,
        childSnapshot.phase
      );
      const childTree = buildSubRunThreadTree(childReplayed.messages);
      return {
        filter,
        cursor: snapshot.cursor,
        phase: replayed.phase,
        messages: replayed.messages,
        childSubRuns: listRootSubRunViews(childTree),
        childContentFingerprint: childTree.rootStreamFingerprint,
      };
    },
    [loadSession]
  );

  const loadAndActivateSession = useCallback(
    async (projectId: string, sessionId: string, subRunPath: string[] = []) => {
      const activationGeneration = ++sessionActivationGenerationRef.current;
      const previousSessionId = activeSessionIdRef.current;
      disconnectSession();
      const loaded = await loadSessionView(sessionId, subRunPath);
      if (activationGeneration !== sessionActivationGenerationRef.current) {
        return;
      }
      dispatch({
        type: 'REPLACE_SESSION_MESSAGES',
        sessionId,
        messages: loaded.messages,
      });
      setActiveSubRunChildren({
        subRuns: loaded.childSubRuns,
        contentFingerprint: loaded.childContentFingerprint,
      });
      // 先写入快照，再切换 active，避免会话切换瞬间渲染空白列表。
      activeSessionIdRef.current = sessionId;
      dispatch({ type: 'SET_ACTIVE', projectId, sessionId });
      dispatch({ type: 'SET_ACTIVE_SUBRUN_PATH', subRunPath });
      phaseRef.current = loaded.phase;
      dispatch({ type: 'SET_PHASE', phase: loaded.phase });
      await connectSession(sessionId, loaded.cursor, loaded.filter);
      if (activationGeneration !== sessionActivationGenerationRef.current) {
        return;
      }
      if (previousSessionId !== sessionId) {
        setModelRefreshKey((value) => value + 1);
      }
    },
    [connectSession, disconnectSession, loadSessionView]
  );

  const refreshSessions = useCallback(
    async (options?: { preferredSessionId?: string | null; preferredSubRunPath?: string[] }) => {
      const activationGeneration = ++sessionActivationGenerationRef.current;
      const previousSessionId = activeSessionIdRef.current;
      const sessionMetas = await listSessionsWithMeta();
      const projects = groupSessionsByProject(sessionMetas);
      const availableSessionIds = sessionMetas.map((meta) => meta.sessionId);
      const preferredSessionId = options?.preferredSessionId;
      const matchedPreferredSessionId = findMatchingSessionId(
        availableSessionIds,
        preferredSessionId
      );
      const matchedActiveSessionId = findMatchingSessionId(
        availableSessionIds,
        activeSessionIdRef.current
      );
      const nextSessionId =
        matchedPreferredSessionId ?? matchedActiveSessionId ?? projects[0]?.sessions[0]?.id ?? null;
      const nextActiveSubRunPath =
        nextSessionId !== null &&
        preferredSessionId !== null &&
        preferredSessionId !== undefined &&
        normalizeSessionIdForCompare(nextSessionId) ===
          normalizeSessionIdForCompare(preferredSessionId)
          ? (options?.preferredSubRunPath ?? [])
          : nextSessionId !== null &&
              activeSessionIdRef.current !== null &&
              normalizeSessionIdForCompare(nextSessionId) ===
                normalizeSessionIdForCompare(activeSessionIdRef.current)
            ? activeSubRunPathRef.current
            : [];
      const nextProjectId =
        projects.find((project) => project.sessions.some((session) => session.id === nextSessionId))
          ?.id ?? null;

      if (nextProjectId && nextSessionId) {
        disconnectSession();
        const loaded = await loadSessionView(nextSessionId, nextActiveSubRunPath);
        if (activationGeneration !== sessionActivationGenerationRef.current) {
          return;
        }
        const hydratedProjects = replaceSessionMessages(projects, nextSessionId, loaded.messages);
        activeSessionIdRef.current = nextSessionId;
        phaseRef.current = loaded.phase;
        setActiveSubRunChildren({
          subRuns: loaded.childSubRuns,
          contentFingerprint: loaded.childContentFingerprint,
        });
        dispatch({
          type: 'INITIALIZE',
          projects: hydratedProjects,
          activeProjectId: nextProjectId,
          activeSessionId: nextSessionId,
          activeSubRunPath: nextActiveSubRunPath,
        });
        dispatch({ type: 'SET_PHASE', phase: loaded.phase });
        await connectSession(nextSessionId, loaded.cursor, loaded.filter);
        if (activationGeneration !== sessionActivationGenerationRef.current) {
          return;
        }
        if (previousSessionId !== nextSessionId) {
          setModelRefreshKey((value) => value + 1);
        }
        return;
      }

      activeSessionIdRef.current = null;
      phaseRef.current = 'idle';
      setActiveSubRunChildren({
        subRuns: [],
        contentFingerprint: '',
      });
      dispatch({
        type: 'INITIALIZE',
        projects,
        activeProjectId: nextProjectId,
        activeSessionId: nextSessionId,
        activeSubRunPath: [],
      });
      dispatch({ type: 'SET_PHASE', phase: 'idle' });
      disconnectSession();
    },
    [connectSession, disconnectSession, listSessionsWithMeta, loadSessionView]
  );

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        await refreshSessions({
          preferredSessionId: initialViewLocationRef.current.sessionId,
          preferredSubRunPath: initialViewLocationRef.current.subRunPath,
        });
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
  const activeSubRunThreadTree = useMemo(
    () => (activeSession ? buildSubRunThreadTree(activeSession.messages) : null),
    [activeSession]
  );
  useEffect(() => {
    if (!activeSubRunThreadTree) {
      return;
    }
    activeSubRunThreadTree.subRuns.forEach((view, subRunId) => {
      subRunTitleCacheRef.current.set(subRunId, view.title);
    });
  }, [activeSubRunThreadTree]);

  useEffect(() => {
    activeSubRunChildren.subRuns.forEach((view) => {
      subRunTitleCacheRef.current.set(view.subRunId, view.title);
    });
  }, [activeSubRunChildren.subRuns]);

  const focusedSubRunId = state.activeSubRunPath[state.activeSubRunPath.length - 1] ?? null;
  const activeSubRunView = focusedSubRunId
    ? (activeSubRunThreadTree?.subRuns.get(focusedSubRunId) ?? null)
    : null;
  const activeSubRunBreadcrumbs = state.activeSubRunPath.map((subRunId) => ({
    subRunId,
    title:
      activeSubRunThreadTree?.subRuns.get(subRunId)?.title ??
      subRunTitleCacheRef.current.get(subRunId) ??
      subRunId,
  }));
  const threadItems =
    activeSubRunView?.threadItems ?? activeSubRunThreadTree?.rootThreadItems ?? [];
  const contentFingerprint = activeSubRunView
    ? `${activeSubRunView.streamFingerprint}|children:${activeSubRunChildren.contentFingerprint}`
    : (activeSubRunThreadTree?.rootStreamFingerprint ?? '');

  useEffect(() => {
    const nextHref = buildSessionViewLocationHref(window.location.href, {
      sessionId: state.activeSessionId,
      subRunPath: state.activeSubRunPath,
    });
    const currentPath = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (nextHref !== currentPath) {
      window.history.replaceState({}, document.title, nextHref);
    }
  }, [state.activeSessionId, state.activeSubRunPath]);

  const handleSessionCatalogEvent = useCallback(
    (event: SessionCatalogEventPayload) => {
      switch (event.event) {
        case 'sessionBranched':
          if (activeSessionIdRef.current === event.data.sourceSessionId) {
            void refreshSessions({ preferredSessionId: event.data.sessionId });
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
      await refreshSessions({ preferredSessionId: created.sessionId });
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
      await refreshSessions({ preferredSessionId: created.sessionId });
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

  const handleOpenSubRun = useCallback(
    async (subRunId: string) => {
      if (!state.activeProjectId || !state.activeSessionId) {
        return;
      }
      const nextSubRunPath = [...activeSubRunPathRef.current, subRunId];
      await loadAndActivateSession(state.activeProjectId, state.activeSessionId, nextSubRunPath);
    },
    [loadAndActivateSession, state.activeProjectId, state.activeSessionId]
  );

  const handleCloseSubRun = useCallback(async () => {
    if (!state.activeProjectId || !state.activeSessionId) {
      return;
    }
    await loadAndActivateSession(state.activeProjectId, state.activeSessionId, []);
  }, [loadAndActivateSession, state.activeProjectId, state.activeSessionId]);

  const handleNavigateSubRunPath = useCallback(
    async (subRunPath: string[]) => {
      if (!state.activeProjectId || !state.activeSessionId) {
        return;
      }
      await loadAndActivateSession(state.activeProjectId, state.activeSessionId, subRunPath);
    },
    [loadAndActivateSession, state.activeProjectId, state.activeSessionId]
  );

  const handleOpenChildSession = useCallback(
    async (childSessionId: string) => {
      const canonicalChildSessionId = normalizeSessionIdForCompare(childSessionId);
      const matchingEntry = state.projects
        .flatMap((project) =>
          project.sessions.map((session) => ({
            projectId: project.id,
            sessionId: session.id,
          }))
        )
        .find((entry) => normalizeSessionIdForCompare(entry.sessionId) === canonicalChildSessionId);

      if (matchingEntry) {
        await loadAndActivateSession(matchingEntry.projectId, matchingEntry.sessionId, []);
        return;
      }
      await refreshSessions({ preferredSessionId: childSessionId });
    },
    [loadAndActivateSession, refreshSessions, state.projects]
  );

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
          await refreshSessions({ preferredSessionId: effectiveSessionId });
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

  // 使用 h-full 而非 h-dvh，因为 WebView2 对 dvh（动态视口高度）的支持不稳定，
  // 会导致桌面端滚动容器高度计算错误，表现为消息列表无法继续下滑。
  return (
    <div className="flex h-full min-h-0 overflow-hidden bg-[var(--app-bg)] text-[var(--text-primary)]">
      {isSidebarOpen && (
        <>
          <div className="flex-none min-w-0 min-h-0" style={{ width: `${sidebarWidth}px` }}>
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
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <Chat
          project={activeProject}
          session={activeSession}
          threadItems={threadItems}
          childSubRuns={activeSubRunChildren.subRuns}
          subRunViews={activeSubRunThreadTree?.subRuns ?? new Map()}
          contentFingerprint={contentFingerprint}
          isSidebarOpen={isSidebarOpen}
          toggleSidebar={toggleSidebar}
          phase={state.phase}
          activeSubRunPath={state.activeSubRunPath}
          activeSubRunTitle={activeSubRunView?.title ?? null}
          activeSubRunBreadcrumbs={activeSubRunBreadcrumbs}
          onOpenSubRun={(subRunId) => {
            void handleOpenSubRun(subRunId);
          }}
          onCloseSubRun={() => {
            void handleCloseSubRun();
          }}
          onNavigateSubRunPath={(subRunPath) => {
            void handleNavigateSubRunPath(subRunPath);
          }}
          onOpenChildSession={handleOpenChildSession}
          onSubmitPrompt={handleSubmit}
          onInterrupt={handleInterrupt}
          onCancelSubRun={cancelSubRun}
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
