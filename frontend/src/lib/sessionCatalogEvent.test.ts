import { describe, expect, it } from 'vitest';

import { normalizeSessionCatalogEvent } from './sessionCatalogEvent';

describe('normalizeSessionCatalogEvent', () => {
  it('accepts sessionBranched payloads', () => {
    const normalized = normalizeSessionCatalogEvent({
      protocolVersion: 1,
      event: 'sessionBranched',
      data: {
        session_id: 'session-2',
        source_session_id: 'session-1',
      },
    });

    expect(normalized).toEqual({
      event: 'sessionBranched',
      data: {
        sessionId: 'session-2',
        sourceSessionId: 'session-1',
      },
    });
  });
});
