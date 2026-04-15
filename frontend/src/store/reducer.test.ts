import { describe, expect, it } from 'vitest';

import { makeInitialState, reducer } from './reducer';
import { createEmptySubRunThreadTree } from '../lib/subRunView';

function makeSessionState() {
  return {
    ...makeInitialState(),
    projects: [
      {
        id: 'project-1',
        name: 'Project',
        workingDir: 'D:/repo',
        isExpanded: true,
        sessions: [
          {
            id: 'session-1',
            projectId: 'project-1',
            title: '新会话',
            createdAt: Date.now(),
            subRunThreadTree: createEmptySubRunThreadTree(),
            messages: [],
          },
        ],
      },
    ],
    activeProjectId: 'project-1',
    activeSessionId: 'session-1',
  };
}

describe('reducer', () => {
  it('clears focused sub-run when switching active session', () => {
    const initial = {
      ...makeSessionState(),
      activeSubRunPath: ['subrun-1', 'subrun-2'],
    };

    const next = reducer(initial, {
      type: 'SET_ACTIVE',
      projectId: 'project-2',
      sessionId: 'session-2',
    });

    expect(next.activeProjectId).toBe('project-2');
    expect(next.activeSessionId).toBe('session-2');
    expect(next.activeSubRunPath).toEqual([]);
  });

  it('pushes and trims nested sub-run path', () => {
    const initial = makeInitialState();

    const pushed = reducer(initial, {
      type: 'PUSH_ACTIVE_SUBRUN',
      subRunId: 'subrun-1',
    });
    const nested = reducer(pushed, {
      type: 'PUSH_ACTIVE_SUBRUN',
      subRunId: 'subrun-2',
    });
    const popped = reducer(nested, { type: 'POP_ACTIVE_SUBRUN' });

    expect(pushed.activeSubRunPath).toEqual(['subrun-1']);
    expect(nested.activeSubRunPath).toEqual(['subrun-1', 'subrun-2']);
    expect(popped.activeSubRunPath).toEqual(['subrun-1']);
  });

  it('appends local messages and keeps the sub-run tree in sync', () => {
    const initial = makeSessionState();

    const next = reducer(initial, {
      type: 'ADD_MESSAGE',
      sessionId: 'session-1',
      message: {
        id: 'child-summary-1',
        kind: 'childSessionNotification',
        agentId: 'agent-child',
        agentProfile: 'repo-inspector',
        subRunId: 'subrun-1',
        childSessionId: 'session-child',
        childRef: {
          agentId: 'agent-child',
          sessionId: 'session-child',
          subRunId: 'subrun-1',
          lineageKind: 'spawn',
          status: 'running',
          openSessionId: 'session-child',
        },
        notificationKind: 'progress_summary',
        status: 'running',
        timestamp: 1,
      },
    });

    const session = next.projects[0].sessions[0];
    expect(session.messages).toHaveLength(1);
    expect(session.subRunThreadTree.subRuns.has('subrun-1')).toBe(true);
  });

  it('replaces session messages with the authoritative conversation projection', () => {
    const initial = makeSessionState();

    const next = reducer(initial, {
      type: 'REPLACE_SESSION_MESSAGES',
      sessionId: 'session-1',
      messages: [
        {
          id: 'assistant-1',
          kind: 'assistant',
          turnId: 'turn-1',
          text: 'hello world',
          reasoningText: 'thinking',
          streaming: false,
          timestamp: 1,
        },
      ],
    });

    const session = next.projects[0].sessions[0];
    expect(session.messages).toEqual([
      expect.objectContaining({
        kind: 'assistant',
        turnId: 'turn-1',
        text: 'hello world',
      }),
    ]);
    expect(session.subRunThreadTree.rootThreadItems).toHaveLength(1);
  });
});
