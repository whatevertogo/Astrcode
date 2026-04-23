import { afterEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
const waitForTauriEnvironmentMock = vi.fn();
const isTauriEnvironmentMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

vi.mock('./tauri', () => ({
  isTauriEnvironment: isTauriEnvironmentMock,
  waitForTauriEnvironment: waitForTauriEnvironmentMock,
}));

function setWindowBootstrap(bootstrap?: {
  isDesktopHost?: boolean;
  token?: string;
  serverOrigin?: string;
}): void {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      __ASTRCODE_BOOTSTRAP__: bootstrap,
    },
  });
}

describe('hostBridge', () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.resetModules();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(globalThis, 'window');
    Reflect.deleteProperty(globalThis, 'navigator');
  });

  it('uses the browser bridge when neither tauri nor bootstrap desktop flag is present', async () => {
    isTauriEnvironmentMock.mockReturnValue(false);
    waitForTauriEnvironmentMock.mockResolvedValue(undefined);
    setWindowBootstrap(undefined);

    const clipboardWriteText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(globalThis, 'navigator', {
      configurable: true,
      value: {
        clipboard: {
          writeText: clipboardWriteText,
        },
      },
    });

    const { getHostBridge } = await import('./hostBridge');

    const bridge = getHostBridge();

    expect(bridge.isDesktopHost).toBe(false);
    expect(bridge.canSelectDirectory).toBe(false);
    expect(bridge.canOpenEditor).toBe(false);
    await expect(bridge.selectDirectory()).resolves.toBeNull();
    await bridge.openConfigInEditor('D:/GitObjectsOwn/Astrcode/docs/issues.md');
    expect(clipboardWriteText).toHaveBeenCalledWith('D:/GitObjectsOwn/Astrcode/docs/issues.md');
    expect(waitForTauriEnvironmentMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('uses the desktop bridge when bootstrap marks the host as desktop', async () => {
    isTauriEnvironmentMock.mockReturnValue(false);
    waitForTauriEnvironmentMock.mockResolvedValue(undefined);
    invokeMock.mockResolvedValueOnce('D:/GitObjectsOwn/Astrcode');
    invokeMock.mockResolvedValueOnce(undefined);
    setWindowBootstrap({
      isDesktopHost: true,
      token: 'desktop-token',
      serverOrigin: 'http://127.0.0.1:62000/',
    });

    const { getHostBridge } = await import('./hostBridge');

    const bridge = getHostBridge();

    expect(bridge.isDesktopHost).toBe(true);
    expect(bridge.canSelectDirectory).toBe(true);
    expect(bridge.canOpenEditor).toBe(true);
    await expect(bridge.selectDirectory()).resolves.toBe('D:/GitObjectsOwn/Astrcode');
    await bridge.openConfigInEditor('D:/GitObjectsOwn/Astrcode/docs/issues.md');
    expect(waitForTauriEnvironmentMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenNthCalledWith(1, 'select_directory');
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'open_config_in_editor', {
      path: 'D:/GitObjectsOwn/Astrcode/docs/issues.md',
    });
  });
});
