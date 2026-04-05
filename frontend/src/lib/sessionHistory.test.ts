import { describe, expect, it } from 'vitest';
import { replaySessionHistory } from './sessionHistory';
import type { AgentEventPayload } from '../types';

describe('replaySessionHistory', () => {
  it('rebuilds message history from agent events and trusts server phase for the final state', () => {
    const events: AgentEventPayload[] = [
      { event: 'sessionStarted', data: { sessionId: 'session-1' } },
      { event: 'userMessage', data: { turnId: 'turn-1', content: 'hello' } },
      { event: 'phaseChanged', data: { turnId: 'turn-1', phase: 'thinking' } },
      { event: 'thinkingDelta', data: { turnId: 'turn-1', delta: '思考中' } },
      { event: 'assistantMessage', data: { turnId: 'turn-1', content: 'world' } },
      {
        event: 'toolCallStart',
        data: {
          turnId: 'turn-1',
          toolCallId: 'tool-1',
          toolName: 'grep',
          args: { pattern: 'hello' },
        },
      },
      {
        event: 'toolCallResult',
        data: {
          turnId: 'turn-1',
          result: {
            toolCallId: 'tool-1',
            toolName: 'grep',
            ok: true,
            output: 'match',
            durationMs: 12,
          },
        },
      },
      {
        event: 'compactApplied',
        data: {
          trigger: 'manual',
          summary: '保留最近一轮',
          preservedRecentTurns: 1,
          turnId: null,
        },
      },
    ];

    const replayed = replaySessionHistory('session-1', events, 'streaming');

    expect(replayed.phase).toBe('streaming');
    expect(replayed.messages.map((message) => message.kind)).toEqual([
      'user',
      'assistant',
      'toolCall',
      'compact',
    ]);
    expect(replayed.messages[0]).toMatchObject({ kind: 'user', text: 'hello' });
    expect(replayed.messages[1]).toMatchObject({
      kind: 'assistant',
      text: 'world',
      reasoningText: '思考中',
    });
    expect(replayed.messages[2]).toMatchObject({
      kind: 'toolCall',
      toolName: 'grep',
      status: 'ok',
      output: 'match',
    });
    expect(replayed.messages[3]).toMatchObject({
      kind: 'compact',
      summary: '保留最近一轮',
    });
  });
});
