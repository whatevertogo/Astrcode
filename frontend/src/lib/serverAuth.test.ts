import { afterEach, describe, expect, it, vi } from 'vitest';

function setWindowLocation(url: string): void {
  const location = new URL(url);
  const history = {
    replaceState: vi.fn(),
  };

  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      location,
      history,
      __ASTRCODE_BOOTSTRAP__: undefined,
    },
  });
}

describe('serverAuth', () => {
  afterEach(() => {
    vi.resetModules();
    Reflect.deleteProperty(globalThis, 'window');
  });

  it('prefers bootstrap token', async () => {
    setWindowLocation('http://127.0.0.1:5173/');
    (
      globalThis as typeof globalThis & {
        window: {
          __ASTRCODE_BOOTSTRAP__?: { token?: string };
        };
      }
    ).window.__ASTRCODE_BOOTSTRAP__ = { token: 'desktop-token' };

    const { getServerAuthToken } = await import('./serverAuth');

    expect(getServerAuthToken()).toBe('desktop-token');
  });

  it('ignores token query parameter', async () => {
    setWindowLocation('http://127.0.0.1:5173/?token=leaked-token');

    const { getServerAuthToken } = await import('./serverAuth');

    expect(getServerAuthToken()).toBeNull();
  });
});
