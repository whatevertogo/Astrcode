import { beforeEach, describe, expect, it, vi } from 'vitest';

const requestJson = vi.fn();

vi.mock('./client', () => ({
  request: vi.fn(),
  requestJson,
}));

describe('forkSession', () => {
  beforeEach(() => {
    requestJson.mockReset();
  });

  it('posts fork request with turnId and returns session meta', async () => {
    requestJson.mockResolvedValue({
      sessionId: 'session-forked',
      workingDir: '.',
      displayName: 'Astrcode',
      title: 'Forked Session',
      createdAt: '2026-04-18T00:00:00Z',
      updatedAt: '2026-04-18T00:00:00Z',
      parentSessionId: 'session-root',
      phase: 'idle',
    });
    const { forkSession } = await import('./sessions');

    const result = await forkSession('session-root', { turnId: 'turn-1' });

    expect(requestJson).toHaveBeenCalledWith('/api/sessions/session-root/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ turnId: 'turn-1', storageSeq: undefined }),
    });
    expect(result.parentSessionId).toBe('session-root');
  });
});
