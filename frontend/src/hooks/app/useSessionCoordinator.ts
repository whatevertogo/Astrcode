import { useCallback, useState, type Dispatch, type MutableRefObject } from 'react';
import { ensureKnownProjects } from '../../lib/knownProjects';
import { groupSessionsByProject, replaceSessionMessages } from '../../store/utils';
import { replaySessionHistory } from '../../lib/sessionHistory';
import { findMatchingSessionId, normalizeSessionIdForCompare } from '../../lib/sessionId';
import { buildSubRunThreadTree, listRootSubRunViews } from '../../lib/subRunView';
import { buildFocusedSubRunFilter, type SessionEventFilterQuery } from '../../lib/sessionView';
import type { Action, Phase, SessionMeta } from '../../types';
import type { SessionViewSnapshot } from '../../types';

type RootSubRunViews = ReturnType<typeof listRootSubRunViews>;

interface ActiveSubRunChildren {
  subRuns: RootSubRunViews;
  contentFingerprint: string;
}

interface RefreshSessionsOptions {
  preferredSessionId?: string | null;
  preferredSubRunPath?: string[];
}

interface UseSessionCoordinatorOptions {
  dispatch: Dispatch<Action>;
  activeSessionIdRef: MutableRefObject<string | null>;
  activeSubRunPathRef: MutableRefObject<string[]>;
  phaseRef: MutableRefObject<Phase>;
  sessionActivationGenerationRef: MutableRefObject<number>;
  loadSessionView: (
    sessionId: string,
    filter?: SessionEventFilterQuery
  ) => Promise<SessionViewSnapshot>;
  listSessionsWithMeta: () => Promise<SessionMeta[]>;
  connectSession: (
    sessionId: string,
    afterEventId?: string | null,
    filter?: SessionEventFilterQuery
  ) => Promise<void>;
  disconnectSession: () => void;
  bumpModelRefreshKey: () => void;
}

export function useSessionCoordinator({
  dispatch,
  activeSessionIdRef,
  activeSubRunPathRef,
  phaseRef,
  sessionActivationGenerationRef,
  loadSessionView: fetchSessionView,
  listSessionsWithMeta,
  connectSession,
  disconnectSession,
  bumpModelRefreshKey,
}: UseSessionCoordinatorOptions) {
  const [activeSubRunChildren, setActiveSubRunChildren] = useState<ActiveSubRunChildren>({
    subRuns: [],
    contentFingerprint: '',
  });

  const loadSessionBundle = useCallback(
    async (sessionId: string, subRunPath: string[]) => {
      const filter = buildFocusedSubRunFilter(subRunPath);
      const snapshot = await fetchSessionView(sessionId, filter);
      const replayed = replaySessionHistory(sessionId, snapshot.focusEvents, snapshot.phase);

      if (!filter?.subRunId) {
        return {
          filter,
          cursor: snapshot.cursor,
          phase: replayed.phase,
          messages: replayed.messages,
          childSubRuns: [] as RootSubRunViews,
          childContentFingerprint: '',
        };
      }

      const childReplayed = replaySessionHistory(
        sessionId,
        snapshot.directChildrenEvents,
        snapshot.phase
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
    [fetchSessionView]
  );

  const loadAndActivateSession = useCallback(
    async (projectId: string, sessionId: string, subRunPath: string[] = []) => {
      const activationGeneration = ++sessionActivationGenerationRef.current;
      const previousSessionId = activeSessionIdRef.current;
      disconnectSession();
      const loaded = await loadSessionBundle(sessionId, subRunPath);
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
        bumpModelRefreshKey();
      }
    },
    [
      activeSessionIdRef,
      bumpModelRefreshKey,
      connectSession,
      disconnectSession,
      dispatch,
      loadSessionBundle,
      phaseRef,
      sessionActivationGenerationRef,
    ]
  );

  const refreshSessions = useCallback(
    async (options?: RefreshSessionsOptions) => {
      const activationGeneration = ++sessionActivationGenerationRef.current;
      const previousSessionId = activeSessionIdRef.current;
      const sessionMetas = await listSessionsWithMeta();
      const knownWorkingDirs = ensureKnownProjects(sessionMetas.map((meta) => meta.workingDir));
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
      const projects = groupSessionsByProject(sessionMetas, knownWorkingDirs, {
        includeSessionIds: [
          ...(matchedPreferredSessionId ? [matchedPreferredSessionId] : []),
          ...(matchedActiveSessionId ? [matchedActiveSessionId] : []),
        ],
      });
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
          ?.id ??
        projects[0]?.id ??
        null;

      if (nextProjectId && nextSessionId) {
        disconnectSession();
        const loaded = await loadSessionBundle(nextSessionId, nextActiveSubRunPath);
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
          bumpModelRefreshKey();
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
    [
      activeSessionIdRef,
      activeSubRunPathRef,
      bumpModelRefreshKey,
      connectSession,
      disconnectSession,
      dispatch,
      listSessionsWithMeta,
      loadSessionBundle,
      phaseRef,
      sessionActivationGenerationRef,
    ]
  );

  return {
    activeSubRunChildren,
    loadAndActivateSession,
    refreshSessions,
  };
}
