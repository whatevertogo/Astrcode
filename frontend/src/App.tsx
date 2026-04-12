import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { reducer as appReducer, makeInitialState } from './store/reducer';
import Sidebar from './components/Sidebar/index';
import Chat from './components/Chat/index';
import SettingsModal from './components/Settings/SettingsModal';
import ConfirmDialog from './components/ConfirmDialog';
import { useAgent } from './hooks/useAgent';
import { useAgentEventHandler } from './hooks/useAgentEventHandler';
import { useSessionCatalogEvents } from './hooks/useSessionCatalogEvents';
import { useSidebarResize } from './hooks/useSidebarResize';
import { useComposerActions, type ConfirmDialogState } from './hooks/app/useComposerActions';
import { useSessionCoordinator } from './hooks/app/useSessionCoordinator';
import { useSubRunNavigation } from './hooks/app/useSubRunNavigation';
import { buildSubRunThreadTree } from './lib/subRunView';
import { buildSessionViewLocationHref, readSessionViewLocation } from './lib/sessionView';
import { cn } from './lib/utils';
import type { SessionCatalogEventPayload } from './types';

const reducer = appReducer;

export default function App() {
  const initialViewLocationRef = useRef(readSessionViewLocation(window.location.href));
  const [state, dispatch] = useReducer(reducer, undefined, makeInitialState);
  const [showSettings, setShowSettings] = useState(false);
  const [modelRefreshKey, setModelRefreshKey] = useState(0);
  // 确认对话框状态（替代 window.confirm）
  const [confirmDialog, setConfirmDialog] = useState<ConfirmDialogState | null>(null);
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

  const bumpModelRefreshKey = useCallback(() => {
    setModelRefreshKey((value) => value + 1);
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
    loadSessionView,
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

  const { activeSubRunChildren, loadAndActivateSession, refreshSessions } = useSessionCoordinator({
    dispatch,
    activeSessionIdRef,
    activeSubRunPathRef,
    phaseRef,
    sessionActivationGenerationRef,
    loadSessionView,
    listSessionsWithMeta,
    connectSession,
    disconnectSession,
    bumpModelRefreshKey,
  });

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

  const handleSetActive = useCallback(
    async (projectId: string, sessionId: string) => {
      try {
        await loadAndActivateSession(projectId, sessionId);
      } catch (error) {
        console.error('Failed to activate session:', error);
      }
    },
    [loadAndActivateSession]
  );

  const { handleOpenSubRun, handleCloseSubRun, handleNavigateSubRunPath, handleOpenChildSession } =
    useSubRunNavigation({
      activeProjectId: state.activeProjectId,
      activeSessionId: state.activeSessionId,
      activeSubRunPath: state.activeSubRunPath,
      projects: state.projects,
      loadAndActivateSession,
      refreshSessions,
    });

  const {
    handleDeleteProject,
    handleDeleteSession,
    handleInterrupt,
    handleNewProject,
    handleNewSession,
    handleSubmit,
  } = useComposerActions({
    activeProject,
    projects: state.projects,
    dispatch,
    phaseRef,
    activeSessionIdRef,
    pendingSubmitSessionRef,
    turnSessionMapRef,
    releasePendingSubmitSession,
    setConfirmDialog,
    refreshSessions,
    createSession,
    submitPrompt,
    interrupt,
    compactSession,
    deleteSession,
    deleteProject,
  });

  const chatContextValue = useMemo(
    () => ({
      projectName: activeProject?.name ?? null,
      sessionId: activeSession?.id ?? null,
      sessionTitle: activeSession?.title ?? null,
      workingDir: activeProject?.workingDir ?? '',
      phase: state.phase,
      activeSubRunPath: state.activeSubRunPath,
      activeSubRunTitle: activeSubRunView?.title ?? null,
      activeSubRunBreadcrumbs,
      isSidebarOpen,
      toggleSidebar,
      onOpenSubRun: handleOpenSubRun,
      onCloseSubRun: handleCloseSubRun,
      onNavigateSubRunPath: handleNavigateSubRunPath,
      onOpenChildSession: handleOpenChildSession,
      onSubmitPrompt: handleSubmit,
      onInterrupt: handleInterrupt,
      onCancelSubRun: cancelSubRun,
      listComposerOptions,
      modelRefreshKey,
      getCurrentModel,
      listAvailableModels,
      setModel,
    }),
    [
      activeProject?.name,
      activeProject?.workingDir,
      activeSession?.id,
      activeSession?.title,
      activeSubRunBreadcrumbs,
      activeSubRunView?.title,
      cancelSubRun,
      getCurrentModel,
      handleCloseSubRun,
      handleInterrupt,
      handleNavigateSubRunPath,
      handleOpenChildSession,
      handleOpenSubRun,
      handleSubmit,
      isSidebarOpen,
      listAvailableModels,
      listComposerOptions,
      modelRefreshKey,
      setModel,
      state.activeSubRunPath,
      state.phase,
      toggleSidebar,
    ]
  );

  // 使用 h-full 而非 h-dvh，因为 WebView2 对 dvh（动态视口高度）的支持不稳定，
  // 会导致桌面端滚动容器高度计算错误，表现为消息列表无法继续下滑。
  return (
    <div className="flex h-full min-h-0 overflow-hidden bg-app-bg text-text-primary">
      {isSidebarOpen && (
        <>
          <div className="min-h-0 min-w-0 flex-none" style={{ width: `${sidebarWidth}px` }}>
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
              onToggleExpand={(projectId) => {
                dispatch({ type: 'TOGGLE_EXPAND', projectId });
              }}
              onNewProject={(workingDir) => {
                void handleNewProject(workingDir);
              }}
              onDeleteProject={(projectId) => {
                handleDeleteProject(projectId);
              }}
              onDeleteSession={(_projectId, sessionId) => {
                handleDeleteSession(sessionId);
              }}
              onOpenSettings={() => setShowSettings(true)}
              onNewSession={() => {
                void handleNewSession();
              }}
            />
          </div>
          <div
            className={cn(
              'relative w-[10px] flex-none cursor-col-resize bg-transparent outline-none before:absolute before:inset-y-0 before:left-1/2 before:w-[1px] before:-translate-x-1/2 before:bg-border hover:before:w-[2px] hover:before:bg-border-strong focus-visible:before:w-[2px] focus-visible:before:bg-border-strong before:transition-all before:duration-150 before:ease-out',
              isResizingSidebar && 'before:w-[2px] before:bg-border-strong'
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
          threadItems={threadItems}
          childSubRuns={activeSubRunChildren.subRuns}
          subRunViews={activeSubRunThreadTree?.subRuns ?? new Map()}
          contentFingerprint={contentFingerprint}
          contextValue={chatContextValue}
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
