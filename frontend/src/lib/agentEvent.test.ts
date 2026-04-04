import { describe, expect, it } from 'vitest';

import { normalizeAgentEvent } from './agentEvent';

describe('normalizeAgentEvent protocol gate', () => {
  it('rejects payload when protocolVersion is missing', () => {
    const normalized = normalizeAgentEvent({
      event: 'phaseChanged',
      data: { phase: 'idle', turnId: null },
    });

    expect(normalized.event).toBe('error');
    if (normalized.event === 'error') {
      expect(normalized.data.code).toBe('invalid_agent_event');
      expect(normalized.data.message).toContain('protocolVersion field is missing');
    }
  });

  it('rejects payload when protocolVersion is incompatible', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 2,
      event: 'phaseChanged',
      data: { phase: 'idle', turnId: null },
    });

    expect(normalized.event).toBe('error');
    if (normalized.event === 'error') {
      expect(normalized.data.message).toContain('unsupported protocolVersion');
    }
  });

  it('accepts payload when protocolVersion is 1', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'modelDelta',
      data: { turnId: 'turn-1', delta: 'hello' },
    });

    expect(normalized).toEqual({
      event: 'modelDelta',
      data: { turnId: 'turn-1', delta: 'hello' },
    });
  });

  it('accepts userMessage payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'userMessage',
      data: { turn_id: 'turn-user', content: 'hello' },
    });

    expect(normalized).toEqual({
      event: 'userMessage',
      data: { turnId: 'turn-user', content: 'hello' },
    });
  });

  it('accepts assistantMessage when content is empty but reasoning exists', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'assistantMessage',
      data: {
        turn_id: 'turn-2',
        content: '',
        reasoning_content: '先检查相关文件。',
      },
    });

    expect(normalized).toEqual({
      event: 'assistantMessage',
      data: {
        turnId: 'turn-2',
        content: '',
        reasoningContent: '先检查相关文件。',
      },
    });
  });

  it('accepts assistantMessage when both content and reasoning are empty', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'assistantMessage',
      data: {
        turn_id: 'turn-3',
        content: '',
      },
    });

    expect(normalized).toEqual({
      event: 'assistantMessage',
      data: {
        turnId: 'turn-3',
        content: '',
      },
    });
  });

  it('accepts empty string deltas for streaming events', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'modelDelta',
      data: {
        turnId: 'turn-empty',
        delta: '',
      },
    });

    expect(normalized).toEqual({
      event: 'modelDelta',
      data: {
        turnId: 'turn-empty',
        delta: '',
      },
    });
  });

  it('accepts toolCallDelta payloads and normalizes stream names', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'toolCallDelta',
      data: {
        turn_id: 'turn-shell',
        tool_call_id: 'call-1',
        tool_name: 'shell',
        stream: 'stderr',
        delta: 'boom\\n',
      },
    });

    expect(normalized).toEqual({
      event: 'toolCallDelta',
      data: {
        turnId: 'turn-shell',
        toolCallId: 'call-1',
        toolName: 'shell',
        stream: 'stderr',
        delta: 'boom\\n',
      },
    });
  });

  it('accepts compactApplied payloads', () => {
    const normalized = normalizeAgentEvent({
      protocolVersion: 1,
      event: 'compactApplied',
      data: {
        turn_id: null,
        trigger: 'manual',
        summary: '保留最近两轮上下文',
        preserved_recent_turns: 2,
      },
    });

    expect(normalized).toEqual({
      event: 'compactApplied',
      data: {
        turnId: null,
        trigger: 'manual',
        summary: '保留最近两轮上下文',
        preservedRecentTurns: 2,
      },
    });
  });
});
