//! # Store Utilities
//!
//! Session message conversion, project grouping, and session message replacement helpers.
//! These were previously at the top of App.tsx and made the file harder to navigate.

import type { Message, Project, SessionMeta, SubRunThreadTree } from '../types';
import { normalizeProjectIdentity } from '../lib/knownProjects';
import { createEmptySubRunThreadTree } from '../lib/subRunView';

interface GroupSessionsOptions {
  includeSessionIds?: string[];
}

function toEpochMs(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function getDirectoryName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || '默认项目';
}

export function groupSessionsByProject(
  sessionMetas: SessionMeta[],
  knownWorkingDirs: string[] = [],
  options: GroupSessionsOptions = {}
): Project[] {
  const visibleSessionIds = new Set(options.includeSessionIds ?? []);
  const projectMap = new Map<string, { project: Project; maxUpdatedAt: number }>();

  for (const meta of sessionMetas) {
    if (meta.parentSessionId && !visibleSessionIds.has(meta.sessionId)) {
      continue;
    }

    const projectId = normalizeProjectIdentity(meta.workingDir) || '__default_project__';
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
      if (!holder.project.workingDir && meta.workingDir) {
        holder.project = {
          ...holder.project,
          name: projectName,
          workingDir: meta.workingDir,
        };
      }
    }

    holder.project.sessions.push({
      id: meta.sessionId,
      projectId,
      title: meta.title || '新会话',
      createdAt,
      updatedAt,
      parentSessionId: meta.parentSessionId,
      messages: [],
      subRunThreadTree: createEmptySubRunThreadTree(),
    });
  }

  for (const workingDir of knownWorkingDirs) {
    const projectId = normalizeProjectIdentity(workingDir);
    if (!projectId || projectMap.has(projectId)) {
      continue;
    }

    projectMap.set(projectId, {
      project: {
        id: projectId,
        name: getDirectoryName(workingDir),
        workingDir,
        isExpanded: true,
        sessions: [],
      },
      maxUpdatedAt: 0,
    });
  }

  const projects = Array.from(projectMap.values());
  projects.sort((a, b) => b.maxUpdatedAt - a.maxUpdatedAt);
  return projects.map((item) => {
    item.project.sessions.sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
    return item.project;
  });
}

export function replaceSessionMessages(
  projects: Project[],
  sessionId: string,
  messages: Message[],
  subRunThreadTree: SubRunThreadTree
): Project[] {
  return projects.map((project) => ({
    ...project,
    sessions: project.sessions.map((session) =>
      session.id === sessionId ? { ...session, messages, subRunThreadTree } : session
    ),
  }));
}
