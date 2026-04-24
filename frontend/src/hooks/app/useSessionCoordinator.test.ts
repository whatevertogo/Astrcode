import { describe, expect, it } from 'vitest';

import { resolveRefreshTargetSelection } from './useSessionCoordinator';

describe('resolveRefreshTargetSelection', () => {
  it('keeps a pending preferred session ahead of a later generic refresh', () => {
    const selection = resolveRefreshTargetSelection({
      availableSessionIds: ['session-old', 'session-new'],
      requestedPreferredSessionId: undefined,
      pendingPreferredSessionId: 'session-new',
      activeSessionId: 'session-old',
      activeSubRunPath: ['subrun-old'],
      fallbackSessionId: 'session-old',
    });

    expect(selection.nextSessionId).toBe('session-new');
    expect(selection.nextActiveSubRunPath).toEqual([]);
  });

  it('preserves the active sub-run path when no preferred session is pending', () => {
    const selection = resolveRefreshTargetSelection({
      availableSessionIds: ['session-old', 'session-new'],
      requestedPreferredSessionId: undefined,
      pendingPreferredSessionId: null,
      activeSessionId: 'session-old',
      activeSubRunPath: ['subrun-old'],
      fallbackSessionId: 'session-new',
    });

    expect(selection.nextSessionId).toBe('session-old');
    expect(selection.nextActiveSubRunPath).toEqual(['subrun-old']);
  });
});
