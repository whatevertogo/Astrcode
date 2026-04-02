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
});
