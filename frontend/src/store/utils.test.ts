import { describe, expect, it } from 'vitest';
import {
  deriveSessionTitleFromMessages,
  groupSessionsByProject,
  replaceSessionMessages,
} from './utils';
import type { SessionMeta } from '../types';
import { buildSubRunThreadTree, createEmptySubRunThreadTree } from '../lib/subRunView';

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

  it('hides child sessions from the default sidebar grouping', () => {
    const projects = groupSessionsByProject([
      buildMeta({ sessionId: 'session-parent' }),
      buildMeta({
        sessionId: 'session-child',
        parentSessionId: 'session-parent',
      }),
    ]);

    expect(projects).toHaveLength(1);
    expect(projects[0]?.sessions.map((session) => session.id)).toEqual(['session-parent']);
  });

  it('includes an explicitly requested child session so direct navigation still works', () => {
    const projects = groupSessionsByProject(
      [
        buildMeta({ sessionId: 'session-parent' }),
        buildMeta({
          sessionId: 'session-child',
          parentSessionId: 'session-parent',
        }),
      ],
      [],
      { includeSessionIds: ['session-child'] }
    );

    expect(projects[0]?.sessions.map((session) => session.id)).toEqual([
      'session-parent',
      'session-child',
    ]);
  });

  it('derives the session title from the first user message', () => {
    const title = deriveSessionTitleFromMessages([
      {
        id: 'user-1',
        kind: 'user',
        text: '你好！这是新的会话标题候选',
        timestamp: 1,
      },
    ]);

    expect(title).toBe('你好！这是新的会话标题候选');
  });

  it('replaces session messages and refreshes the derived title', () => {
    const projects = [
      {
        id: 'project-1',
        name: 'Repo',
        workingDir: 'D:\\Repo',
        isExpanded: true,
        sessions: [
          {
            id: 'session-1',
            projectId: 'project-1',
            title: '新会话',
            createdAt: 1,
            messages: [],
            subRunThreadTree: createEmptySubRunThreadTree(),
          },
        ],
      },
    ];
    const messages = [
      {
        id: 'user-1',
        kind: 'user' as const,
        text: '你好',
        timestamp: 1,
      },
      {
        id: 'assistant-1',
        kind: 'assistant' as const,
        text: '你好！有什么我可以帮你的吗？',
        reasoningText: '',
        streaming: false,
        timestamp: 2,
      },
    ];

    const next = replaceSessionMessages(
      projects,
      'session-1',
      messages,
      buildSubRunThreadTree(messages)
    );

    expect(next[0]?.sessions[0]?.title).toBe('你好');
  });
});
