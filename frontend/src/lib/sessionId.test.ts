import { describe, expect, it } from 'vitest';

import { findMatchingSessionId, normalizeSessionIdForCompare } from './sessionId';

describe('sessionId helpers', () => {
  it('normalizes optional session- prefixes for comparisons', () => {
    expect(normalizeSessionIdForCompare('session-2026-04-09-abc')).toBe('2026-04-09-abc');
    expect(normalizeSessionIdForCompare('2026-04-09-abc')).toBe('2026-04-09-abc');
  });

  it('finds a matching session id regardless of prefix shape', () => {
    expect(findMatchingSessionId(['2026-04-09-abc'], 'session-2026-04-09-abc')).toBe(
      '2026-04-09-abc'
    );
    expect(findMatchingSessionId(['session-2026-04-09-abc'], '2026-04-09-abc')).toBe(
      'session-2026-04-09-abc'
    );
  });
});
