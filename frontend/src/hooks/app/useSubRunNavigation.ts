import { useCallback } from 'react';
import { normalizeSessionIdForCompare } from '../../lib/sessionId';
import type { Project } from '../../types';

interface UseSubRunNavigationOptions {
  activeProjectId: string | null;
  activeSessionId: string | null;
  activeSubRunPath: string[];
  projects: Project[];
  loadAndActivateSession: (
    projectId: string,
    sessionId: string,
    subRunPath?: string[]
  ) => Promise<void>;
  refreshSessions: (options?: { preferredSessionId?: string | null }) => Promise<void>;
}

export function useSubRunNavigation({
  activeProjectId,
  activeSessionId,
  activeSubRunPath,
  projects,
  loadAndActivateSession,
  refreshSessions,
}: UseSubRunNavigationOptions) {
  const handleOpenSubRun = useCallback(
    async (subRunId: string) => {
      if (!activeProjectId || !activeSessionId) {
        return;
      }
      await loadAndActivateSession(activeProjectId, activeSessionId, [
        ...activeSubRunPath,
        subRunId,
      ]);
    },
    [activeProjectId, activeSessionId, activeSubRunPath, loadAndActivateSession]
  );

  const handleCloseSubRun = useCallback(async () => {
    if (!activeProjectId || !activeSessionId) {
      return;
    }
    await loadAndActivateSession(activeProjectId, activeSessionId, []);
  }, [activeProjectId, activeSessionId, loadAndActivateSession]);

  const handleNavigateSubRunPath = useCallback(
    async (subRunPath: string[]) => {
      if (!activeProjectId || !activeSessionId) {
        return;
      }
      await loadAndActivateSession(activeProjectId, activeSessionId, subRunPath);
    },
    [activeProjectId, activeSessionId, loadAndActivateSession]
  );

  const handleOpenChildSession = useCallback(
    async (childSessionId: string) => {
      const canonicalChildSessionId = normalizeSessionIdForCompare(childSessionId);
      const matchingEntry = projects
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
    [loadAndActivateSession, projects, refreshSessions]
  );

  return {
    handleCloseSubRun,
    handleNavigateSubRunPath,
    handleOpenChildSession,
    handleOpenSubRun,
  };
}
