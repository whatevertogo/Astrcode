import { describe, expect, it } from 'vitest';

import type { Message } from '../types';
import { resolveForkTurnIdFromMessage } from './sessionFork';

const baseControl = {
  phase: 'idle' as const,
  canSubmitPrompt: true,
  canRequestCompact: true,
  compactPending: false,
  compacting: false,
};

describe('resolveForkTurnIdFromMessage', () => {
  it('returns turnId for stable root conversation messages', () => {
    const message: Message = {
      id: 'msg-user-1',
      kind: 'user',
      turnId: 'turn-1',
      text: 'hello',
      timestamp: 1,
    };

    expect(resolveForkTurnIdFromMessage(message, baseControl)).toBe('turn-1');
  });

  it('hides fork for messages without stable turn ids', () => {
    const message: Message = {
      id: 'msg-child-1',
      kind: 'childSessionNotification',
      childRef: {
        agentId: 'agent-child',
        sessionId: 'session-child',
        subRunId: 'subrun-child',
        lineageKind: 'spawn',
        status: 'running',
        openSessionId: 'session-child',
      },
      notificationKind: 'started',
      status: 'running',
      timestamp: 1,
    };

    expect(resolveForkTurnIdFromMessage(message, baseControl)).toBeNull();
  });

  it('hides fork for the active unfinished turn', () => {
    const message: Message = {
      id: 'msg-assistant-1',
      kind: 'assistant',
      turnId: 'turn-active',
      text: 'streaming',
      reasoningText: '',
      streaming: true,
      timestamp: 1,
    };

    expect(
      resolveForkTurnIdFromMessage(message, {
        ...baseControl,
        phase: 'streaming',
        activeTurnId: 'turn-active',
      })
    ).toBeNull();
  });
});
