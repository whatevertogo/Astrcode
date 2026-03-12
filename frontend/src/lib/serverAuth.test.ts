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
      setTimeout: globalThis.setTimeout.bind(globalThis),
      clearTimeout: globalThis.clearTimeout.bind(globalThis),
      __ASTRCODE_BOOTSTRAP__: undefined,
    },
  });
}

describe('serverAuth', () => {
  afterEach(() => {
    vi.resetModules();
    vi.unmock('./tauri');
    vi.unstubAllGlobals();
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

  it('waits for desktop bootstrap before first API request', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => true,
    }));
    setWindowLocation('http://127.0.0.1:5173/');

    const desktopWindow = (
      globalThis as typeof globalThis & {
        window: {
          __ASTRCODE_BOOTSTRAP__?: { token?: string; serverOrigin?: string };
        };
      }
    ).window;

    globalThis.setTimeout(() => {
      desktopWindow.__ASTRCODE_BOOTSTRAP__ = {
        token: 'desktop-token',
        serverOrigin: 'http://127.0.0.1:62000/',
      };
    }, 10);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('desktop-token');
    expect(getServerOrigin()).toBe('http://127.0.0.1:62000');
  });

  it('waits for bootstrap on tauri localhost origins before the first API request', async () => {
    setWindowLocation('https://tauri.localhost/');

    const desktopWindow = (
      globalThis as typeof globalThis & {
        window: {
          __ASTRCODE_BOOTSTRAP__?: { token?: string; serverOrigin?: string };
        };
      }
    ).window;

    globalThis.setTimeout(() => {
      desktopWindow.__ASTRCODE_BOOTSTRAP__ = {
        token: 'packaged-token',
        serverOrigin: 'http://127.0.0.1:63000/',
      };
    }, 10);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('packaged-token');
    expect(getServerOrigin()).toBe('http://127.0.0.1:63000');
  });

  it('hydrates browser dev bootstrap from the vite bridge', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        token: 'dev-token',
        serverOrigin: 'http://127.0.0.1:64000/',
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(fetchMock).toHaveBeenCalledWith('/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(getServerAuthToken()).toBe('dev-token');
    expect(getServerOrigin()).toBe('http://127.0.0.1:64000');
  });
});
