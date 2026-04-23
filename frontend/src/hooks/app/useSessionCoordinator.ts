import { useCallback, useState, type Dispatch, type MutableRefObject } from 'react';
import { ensureKnownProjects } from '../../lib/knownProjects';
import { groupSessionsByProject, replaceSessionMessages } from '../../store/utils';
import { findMatchingSessionId, normalizeSessionIdForCompare } from '../../lib/sessionId';
import { buildFocusedSubRunFilter, type SessionEventFilterQuery } from '../../lib/sessionView';
import type {
  Action,
  ConversationControlState,
  ConversationStepProgress,
  Phase,
  SessionMeta,
  SubRunViewData,
} from '../../types';
import type { ConversationViewProjection } from '../../lib/api/conversation';

interface ActiveSubRunChildren {
  subRuns: SubRunViewData[];
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
  loadConversationView: (
    sessionId: string,
    filter?: SessionEventFilterQuery
  ) => Promise<ConversationViewProjection>;
  listSessionsWithMeta: () => Promise<SessionMeta[]>;
  connectSession: (
    sessionId: string,
    afterEventId?: string | null,
    filter?: SessionEventFilterQuery,
    onProjection?: (projection: ConversationViewProjection) => void
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
  loadConversationView,
  listSessionsWithMeta,
  connectSession,
  disconnectSession,
  bumpModelRefreshKey,
}: UseSessionCoordinatorOptions) {
  const [activeSubRunChildren, setActiveSubRunChildren] = useState<ActiveSubRunChildren>({
    subRuns: [],
    contentFingerprint: '',
  });
  const [activeConversationControl, setActiveConversationControl] =
    useState<ConversationControlState | null>(null);
  const [activeConversationStepProgress, setActiveConversationStepProgress] =
    useState<ConversationStepProgress>({
      durable: null,
      live: null,
    });

  const loadSessionBundle = useCallback(
    async (sessionId: string, subRunPath: string[]) => {
      const filter = buildFocusedSubRunFilter(subRunPath);
      const projection = await loadConversationView(sessionId, filter);
      return {
        filter,
        cursor: projection.cursor,
        phase: projection.phase,
        control: projection.control,
        stepProgress: projection.stepProgress,
        messages: projection.messages,
        messageTree: projection.messageTree,
        messageFingerprint: projection.messageFingerprint,
        childSubRuns: projection.childSubRuns,
        childContentFingerprint: projection.childFingerprint,
      };
    },
    [loadConversationView]
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
        subRunThreadTree: loaded.messageTree,
      });
      setActiveSubRunChildren({
        subRuns: loaded.childSubRuns,
        contentFingerprint: loaded.childContentFingerprint,
      });
      setActiveConversationControl(loaded.control);
      setActiveConversationStepProgress(loaded.stepProgress);
      // 先写入快照，再切换 active，避免会话切换瞬间渲染空白列表。
      activeSessionIdRef.current = sessionId;
      dispatch({ type: 'SET_ACTIVE', projectId, sessionId });
      dispatch({ type: 'SET_ACTIVE_SUBRUN_PATH', subRunPath });
      phaseRef.current = loaded.phase;
      dispatch({ type: 'SET_PHASE', phase: loaded.phase });
      await connectSession(sessionId, loaded.cursor, loaded.filter, (projection) => {
        if (projection.messageFingerprint !== loaded.messageFingerprint) {
          dispatch({
            type: 'REPLACE_SESSION_MESSAGES',
            sessionId,
            messages: projection.messages,
            subRunThreadTree: projection.messageTree,
          });
          loaded.messageFingerprint = projection.messageFingerprint;
        }
        if (projection.childFingerprint !== loaded.childContentFingerprint) {
          setActiveSubRunChildren({
            subRuns: projection.childSubRuns,
            contentFingerprint: projection.childFingerprint,
          });
          loaded.childContentFingerprint = projection.childFingerprint;
        }
        if (phaseRef.current !== projection.phase) {
          phaseRef.current = projection.phase;
          dispatch({ type: 'SET_PHASE', phase: projection.phase });
        }
        setActiveConversationControl(projection.control);
        setActiveConversationStepProgress(projection.stepProgress);
      });
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
        const hydratedProjects = replaceSessionMessages(
          projects,
          nextSessionId,
          loaded.messages,
          loaded.messageTree
        );
        activeSessionIdRef.current = nextSessionId;
        phaseRef.current = loaded.phase;
        setActiveSubRunChildren({
          subRuns: loaded.childSubRuns,
          contentFingerprint: loaded.childContentFingerprint,
        });
        setActiveConversationControl(loaded.control);
        setActiveConversationStepProgress(loaded.stepProgress);
        dispatch({
          type: 'INITIALIZE',
          projects: hydratedProjects,
          activeProjectId: nextProjectId,
          activeSessionId: nextSessionId,
          activeSubRunPath: nextActiveSubRunPath,
        });
        dispatch({ type: 'SET_PHASE', phase: loaded.phase });
        await connectSession(nextSessionId, loaded.cursor, loaded.filter, (projection) => {
          if (projection.messageFingerprint !== loaded.messageFingerprint) {
            dispatch({
              type: 'REPLACE_SESSION_MESSAGES',
              sessionId: nextSessionId,
              messages: projection.messages,
              subRunThreadTree: projection.messageTree,
            });
            loaded.messageFingerprint = projection.messageFingerprint;
          }
          if (projection.childFingerprint !== loaded.childContentFingerprint) {
            setActiveSubRunChildren({
              subRuns: projection.childSubRuns,
              contentFingerprint: projection.childFingerprint,
            });
            loaded.childContentFingerprint = projection.childFingerprint;
          }
          if (phaseRef.current !== projection.phase) {
            phaseRef.current = projection.phase;
            dispatch({ type: 'SET_PHASE', phase: projection.phase });
          }
          setActiveConversationControl(projection.control);
          setActiveConversationStepProgress(projection.stepProgress);
        });
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
      setActiveConversationControl(null);
      setActiveConversationStepProgress({
        durable: null,
        live: null,
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
    activeConversationControl,
    activeConversationStepProgress,
    loadAndActivateSession,
    refreshSessions,
  };
}
