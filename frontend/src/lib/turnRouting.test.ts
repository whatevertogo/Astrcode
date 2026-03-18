import { describe, expect, it } from 'vitest';

import { releaseTurnMapping, resolveSessionForTurn } from './turnRouting';

describe('turnRouting', () => {
  it('uses queued session for first event of a turn', () => {
    const map: Record<string, string> = {};
    const pending = ['session-a'];

    const sid = resolveSessionForTurn(map, pending, 'turn-1', 'session-z');

    expect(sid).toBe('session-a');
    expect(map['turn-1']).toBe('session-a');
    expect(pending).toEqual(['session-a']);
  });

  it('reuses mapped session for subsequent events', () => {
    const map: Record<string, string> = { 'turn-1': 'session-a' };
    const pending: string[] = [];

    const sid = resolveSessionForTurn(map, pending, 'turn-1', 'session-z');

    expect(sid).toBe('session-a');
  });

  it('falls back to active session when no queue entry exists', () => {
    const map: Record<string, string> = {};
    const pending: string[] = [];

    const sid = resolveSessionForTurn(map, pending, 'turn-2', 'session-active');

    expect(sid).toBe('session-active');
    expect(map['turn-2']).toBe('session-active');
  });

  it('does not consume queue when resolving the same turn repeatedly', () => {
    const map: Record<string, string> = {};
    const pending = ['session-a'];

    const first = resolveSessionForTurn(map, pending, 'turn-3', 'session-z');
    const second = resolveSessionForTurn(map, pending, 'turn-3', 'session-z');

    expect(first).toBe('session-a');
    expect(second).toBe('session-a');
    expect(pending).toEqual(['session-a']);
  });

  it('releases turn mapping on completion', () => {
    const map: Record<string, string> = { 'turn-1': 'session-a' };

    releaseTurnMapping(map, 'turn-1');

    expect(map['turn-1']).toBeUndefined();
  });
});
