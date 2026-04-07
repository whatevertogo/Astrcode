import { describe, expect, it } from 'vitest';

import {
  buildSessionEventQueryString,
  buildFocusedSubRunFilter,
  buildSubRunChildrenFilter,
  buildSubRunSelfFilter,
  buildSessionViewLocationHref,
  readSessionViewLocation,
} from './sessionView';

describe('sessionView helpers', () => {
  it('builds self and direct-children filters from the focused sub-run context', () => {
    expect(buildFocusedSubRunFilter([])).toBeUndefined();
    expect(buildFocusedSubRunFilter(['subrun-a', 'subrun-b'])).toEqual({
      subRunId: 'subrun-b',
      scope: 'self',
    });
    expect(buildSubRunSelfFilter('subrun-a')).toEqual({
      subRunId: 'subrun-a',
      scope: 'self',
    });
    expect(buildSubRunChildrenFilter('subrun-a')).toEqual({
      subRunId: 'subrun-a',
      scope: 'directChildren',
    });
  });

  it('serializes history and SSE query parameters with a shared builder', () => {
    expect(
      buildSessionEventQueryString({
        afterEventId: '12.3',
        filter: {
          subRunId: 'subrun-a',
          scope: 'directChildren',
        },
      })
    ).toBe('?afterEventId=12.3&subRunId=subrun-a&scope=directChildren');
  });

  it('round-trips session and sub-run location state through the URL', () => {
    const nextHref = buildSessionViewLocationHref('http://localhost:1420/?foo=bar#hash', {
      sessionId: 'session-1',
      subRunPath: ['subrun-a', 'subrun-b'],
    });

    expect(nextHref).toBe('/?foo=bar&sessionId=session-1&subRunPath=subrun-a%2Csubrun-b#hash');
    expect(readSessionViewLocation(`http://localhost:1420${nextHref}`)).toEqual({
      sessionId: 'session-1',
      subRunPath: ['subrun-a', 'subrun-b'],
    });
  });
});
