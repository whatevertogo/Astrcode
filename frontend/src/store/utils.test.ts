import { describe, expect, it } from 'vitest';
import { groupSessionsByProject } from './utils';
import type { SessionMeta } from '../types';

function buildMeta(overrides: Partial<SessionMeta>): SessionMeta {
  return {
    sessionId: 'session-1',
    workingDir: 'D:\\Repo',
    displayName: 'Repo',
    title: '新会话',
    createdAt: '2026-04-13T08:00:00.000Z',
    updatedAt: '2026-04-13T09:00:00.000Z',
    phase: 'idle',
    ...overrides,
  };
}

describe('groupSessionsByProject', () => {
  it('merges equivalent windows paths into one project bucket', () => {
    const projects = groupSessionsByProject([
      buildMeta({ sessionId: 'session-a', workingDir: 'D:\\Repo', displayName: 'Repo' }),
      buildMeta({ sessionId: 'session-b', workingDir: 'd:/repo/', displayName: 'repo' }),
    ]);

    expect(projects).toHaveLength(1);
    expect(projects[0]?.sessions.map((session) => session.id)).toEqual(['session-a', 'session-b']);
  });

  it('keeps remembered projects even when there are no sessions', () => {
    const projects = groupSessionsByProject([], ['D:\\Alpha', 'D:\\Beta']);

    expect(projects.map((project) => project.workingDir)).toEqual(['D:\\Alpha', 'D:\\Beta']);
    expect(projects.every((project) => project.sessions.length === 0)).toBe(true);
  });
});
