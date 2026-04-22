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

    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          ok: true,
          token: 'session-token',
          expiresAtMs: Date.now() + 60_000,
        }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('session-token');
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

    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          ok: true,
          token: 'desktop-session',
          expiresAtMs: Date.now() + 60_000,
        }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('desktop-session');
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

    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          ok: true,
          token: 'packaged-session',
          expiresAtMs: Date.now() + 60_000,
        }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('packaged-session');
    expect(getServerOrigin()).toBe('http://127.0.0.1:63000');
  });

  it('hydrates browser dev bootstrap from the vite bridge', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token',
            serverOrigin: 'http://127.0.0.1:64000/',
          }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            ok: true,
            token: 'dev-session',
            expiresAtMs: Date.now() + 60_000,
          }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(fetchMock).toHaveBeenCalledWith('/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenCalledWith('http://127.0.0.1:64000/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token' }),
    });
    expect(getServerAuthToken()).toBe('dev-session');
    expect(getServerOrigin()).toBe('http://127.0.0.1:64000');
  });

  it('fails fast when the vite bridge reports that run-info is unavailable', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi.fn().mockResolvedValue({
      ok: false,
      status: 503,
      statusText: 'Service Unavailable',
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    await expect(ensureServerSession()).rejects.toThrow(
      '浏览器前端尚未获取到本地服务 bootstrap 信息，请确认 astrcode-server 已启动。'
    );
    expect(getServerAuthToken()).toBeNull();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
  });

  it('rejects incomplete browser bootstrap payloads without attempting token exchange', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          serverOrigin: 'http://127.0.0.1:64000/',
        }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    await expect(ensureServerSession()).rejects.toThrow(
      '浏览器 bootstrap 返回的数据不完整（缺少 token）。'
    );
    expect(getServerAuthToken()).toBeNull();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
  });

  it('lets concurrent callers fail behind one shared unavailable-bridge bootstrap flow', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi.fn().mockResolvedValue({
      ok: false,
      status: 503,
      statusText: 'Service Unavailable',
    });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    const results = await Promise.allSettled([ensureServerSession(), ensureServerSession()]);

    expect(results).toHaveLength(2);
    for (const result of results) {
      expect(result.status).toBe('rejected');
      if (result.status === 'rejected') {
        expect(result.reason).toBeInstanceOf(Error);
        expect((result.reason as Error).message).toBe(
          '浏览器前端尚未获取到本地服务 bootstrap 信息，请确认 astrcode-server 已启动。'
        );
      }
    }
    expect(getServerAuthToken()).toBeNull();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
  });

  it('recovers once the vite bridge becomes available after an earlier bootstrap failure', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: false,
        status: 503,
        statusText: 'Service Unavailable',
      })
      .mockResolvedValueOnce({
        ok: false,
        status: 503,
        statusText: 'Service Unavailable',
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-recovered',
            serverOrigin: 'http://127.0.0.1:65200/',
          }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            ok: true,
            token: 'recovered-after-bridge-ready',
            expiresAtMs: Date.now() + 60_000,
          }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await expect(ensureServerSession()).rejects.toThrow(
      '浏览器前端尚未获取到本地服务 bootstrap 信息，请确认 astrcode-server 已启动。'
    );

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('recovered-after-bridge-ready');
    expect(getServerOrigin()).toBe('http://127.0.0.1:65200');
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(fetchMock).toHaveBeenNthCalledWith(3, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(4, 'http://127.0.0.1:65200/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token-recovered' }),
    });
  });

  it('retries with a fresh bootstrap token after exchange failure consumes the first one', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');

    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-1',
            serverOrigin: 'http://127.0.0.1:65000/',
          }),
      })
      .mockResolvedValueOnce({
        ok: false,
        status: 401,
        statusText: 'Unauthorized',
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-2',
            serverOrigin: 'http://127.0.0.1:65000/',
          }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            ok: true,
            token: 'recovered-session',
            expiresAtMs: Date.now() + 60_000,
          }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('recovered-session');
    expect(fetchMock).toHaveBeenNthCalledWith(2, 'http://127.0.0.1:65000/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token-1' }),
    });
    expect(fetchMock).toHaveBeenNthCalledWith(4, 'http://127.0.0.1:65000/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token-2' }),
    });
  });

  it('recovers when a complete browser bootstrap payload points to an unreachable origin', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');

    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'fake-live-pid-token',
            serverOrigin: 'http://127.0.0.1:65500/',
          }),
      })
      .mockRejectedValueOnce(new Error('connect ECONNREFUSED 127.0.0.1:65500'))
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-2',
            serverOrigin: 'http://127.0.0.1:65010/',
          }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            ok: true,
            token: 'recovered-after-unreachable-origin',
            expiresAtMs: Date.now() + 60_000,
          }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken, getServerOrigin } =
      await import('./serverAuth');

    await ensureServerSession();

    expect(getServerAuthToken()).toBe('recovered-after-unreachable-origin');
    expect(getServerOrigin()).toBe('http://127.0.0.1:65010');
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(2, 'http://127.0.0.1:65500/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'fake-live-pid-token' }),
    });
    expect(fetchMock).toHaveBeenNthCalledWith(3, '/__astrcode__/run-info', {
      cache: 'no-store',
    });
    expect(fetchMock).toHaveBeenNthCalledWith(4, 'http://127.0.0.1:65010/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token-2' }),
    });
  });

  it('lets concurrent callers recover behind one retried bootstrap flow', async () => {
    vi.doMock('./tauri', () => ({
      isTauriEnvironment: () => false,
    }));
    setWindowLocation('http://127.0.0.1:5173/');

    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-1',
            serverOrigin: 'http://127.0.0.1:65100/',
          }),
      })
      .mockResolvedValueOnce({
        ok: false,
        status: 401,
        statusText: 'Unauthorized',
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            token: 'bootstrap-token-2',
            serverOrigin: 'http://127.0.0.1:65100/',
          }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () =>
          Promise.resolve({
            ok: true,
            token: 'recovered-shared-session',
            expiresAtMs: Date.now() + 60_000,
          }),
      });
    vi.stubGlobal('fetch', fetchMock);

    const { ensureServerSession, getServerAuthToken } = await import('./serverAuth');

    await Promise.all([ensureServerSession(), ensureServerSession()]);

    expect(getServerAuthToken()).toBe('recovered-shared-session');
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(fetchMock).toHaveBeenNthCalledWith(4, 'http://127.0.0.1:65100/api/auth/exchange', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ token: 'bootstrap-token-2' }),
    });
  });
});
