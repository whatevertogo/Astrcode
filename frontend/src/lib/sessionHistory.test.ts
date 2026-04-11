import { describe, expect, it } from 'vitest';
import { replaySessionHistory } from './sessionHistory';
import type { AgentEventPayload } from '../types';

function makeParentSummaryProjectionEventsFixture(): AgentEventPayload[] {
  return [
    { event: 'sessionStarted', data: { sessionId: 'session-1' } },
    {
      event: 'childSessionNotification',
      data: {
        turnId: 'turn-root',
        agentId: 'agent-child',
        parentTurnId: 'turn-root',
        subRunId: 'subrun-child',
        executionId: 'execution-child-1',
        childRef: {
          agentId: 'agent-child',
          sessionId: 'session-1',
          subRunId: 'subrun-child',
          executionId: 'execution-child-1',
          lineageKind: 'spawn',
          status: 'idle',
          openSessionId: 'session-child-1',
        },
        kind: 'delivered',
        summary: '子会话已完成摘要',
        status: 'idle',
        sourceToolCallId: 'call-child-1',
        finalReplyExcerpt: '子会话最终回复摘录',
      },
    },
  ];
}

function makeLegacyRejectionEventsFixture(): AgentEventPayload[] {
  return [
    { event: 'sessionStarted', data: { sessionId: 'session-legacy' } },
    {
      event: 'error',
      data: {
        turnId: 'turn-legacy',
        code: 'unsupported_legacy_shared_history',
        message: '该会话使用旧共享历史结构，必须升级后才能继续查看。',
      },
    },
  ];
}

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

  it('replays parent summary facts with child-session entry fields and renders legacy rejection as error text', () => {
    const summaryReplay = replaySessionHistory(
      'session-1',
      makeParentSummaryProjectionEventsFixture(),
      'idle'
    );
    const summaryMessage = summaryReplay.messages.find(
      (message) => message.kind === 'childSessionNotification'
    );

    expect(summaryMessage).toMatchObject({
      kind: 'childSessionNotification',
      subRunId: 'subrun-child',
      executionId: 'execution-child-1',
      childSessionId: 'session-child-1',
      childRef: {
        agentId: 'agent-child',
        executionId: 'execution-child-1',
        openSessionId: 'session-child-1',
      },
      notificationKind: 'delivered',
      status: 'idle',
      summary: '子会话已完成摘要',
      finalReplyExcerpt: '子会话最终回复摘录',
    });
    expect(summaryReplay.messages.some((message) => message.kind === 'user')).toBe(false);

    const legacyReplay = replaySessionHistory(
      'session-legacy',
      makeLegacyRejectionEventsFixture(),
      'idle'
    );
    expect(legacyReplay.messages).toHaveLength(1);
    expect(legacyReplay.messages[0]).toMatchObject({
      kind: 'assistant',
      text: '错误：该会话使用旧共享历史结构，必须升级后才能继续查看。',
    });
  });
});
