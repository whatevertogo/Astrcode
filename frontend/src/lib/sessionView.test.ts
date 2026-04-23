import { describe, expect, it } from 'vitest';
import { makeInitialState } from '../store/reducer';

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

  it('drops sessionId and subRunPath when the active session is empty', () => {
    const nextHref = buildSessionViewLocationHref(
      'http://localhost:1420/?foo=bar&sessionId=session-1&subRunPath=subrun-a%2Csubrun-b#hash',
      {
        sessionId: null,
        subRunPath: ['subrun-a', 'subrun-b'],
      }
    );

    expect(nextHref).toBe('/?foo=bar#hash');
    expect(readSessionViewLocation(`http://localhost:1420${nextHref}`)).toEqual({
      sessionId: null,
      subRunPath: [],
    });
  });

  it('matches App startup sync and clears a deep link before session hydration', () => {
    const initialState = makeInitialState();
    const nextHref = buildSessionViewLocationHref(
      'http://localhost:1420/?sessionId=2026-04-22T03-16-44-c5838d32',
      {
        sessionId: initialState.activeSessionId,
        subRunPath: initialState.activeSubRunPath,
      }
    );

    expect(initialState.activeSessionId).toBeNull();
    expect(initialState.activeSubRunPath).toEqual([]);
    expect(nextHref).toBe('/');
  });
});
