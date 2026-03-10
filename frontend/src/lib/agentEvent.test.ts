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
});
