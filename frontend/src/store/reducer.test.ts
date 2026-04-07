import { describe, expect, it } from 'vitest';

import { makeInitialState, reducer } from './reducer';

describe('app reducer user message sync', () => {
  it('upserts a user message by turn id instead of duplicating it', () => {
    const initial = {
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
              messages: [
                {
                  id: 'user-1',
                  kind: 'user' as const,
                  turnId: 'turn-1',
                  text: 'hello',
                  timestamp: 123,
                },
              ],
            },
          ],
        },
      ],
      activeProjectId: 'project-1',
      activeSessionId: 'session-1',
    };

    const next = reducer(initial, {
      type: 'UPSERT_USER_MESSAGE',
      sessionId: 'session-1',
      turnId: 'turn-1',
      content: 'hello',
    });

    const messages = next.projects[0].sessions[0].messages;
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatchObject({
      id: 'user-1',
      kind: 'user',
      turnId: 'turn-1',
      text: 'hello',
      timestamp: 123,
    });
  });

  it('clears focused sub-run when switching active session', () => {
    const initial = {
      ...makeInitialState(),
      activeProjectId: 'project-1',
      activeSessionId: 'session-1',
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

  it('upserts prompt metrics by turn id and step index instead of duplicating the card', () => {
    const initial = {
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
              messages: [
                {
                  id: 'metrics-1',
                  kind: 'promptMetrics' as const,
                  turnId: 'turn-1',
                  stepIndex: 0,
                  estimatedTokens: 1200,
                  contextWindow: 200000,
                  effectiveWindow: 180000,
                  thresholdTokens: 162000,
                  truncatedToolResults: 0,
                  timestamp: 123,
                },
              ],
            },
          ],
        },
      ],
      activeProjectId: 'project-1',
      activeSessionId: 'session-1',
    };

    const next = reducer(initial, {
      type: 'UPSERT_PROMPT_METRICS',
      sessionId: 'session-1',
      turnId: 'turn-1',
      stepIndex: 0,
      estimatedTokens: 1400,
      contextWindow: 200000,
      effectiveWindow: 180000,
      thresholdTokens: 162000,
      truncatedToolResults: 1,
      providerInputTokens: 1000,
      providerOutputTokens: 120,
      cacheCreationInputTokens: 900,
      cacheReadInputTokens: 800,
    });

    const messages = next.projects[0].sessions[0].messages;
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatchObject({
      id: 'metrics-1',
      kind: 'promptMetrics',
      turnId: 'turn-1',
      stepIndex: 0,
      estimatedTokens: 1400,
      truncatedToolResults: 1,
      providerInputTokens: 1000,
      providerOutputTokens: 120,
      cacheCreationInputTokens: 900,
      cacheReadInputTokens: 800,
      timestamp: 123,
    });
  });
});
