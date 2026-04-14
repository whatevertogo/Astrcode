import { invoke } from '@tauri-apps/api/core';
import { isTauriEnvironment, waitForTauriEnvironment } from './tauri';

declare global {
  interface Window {
    __ASTRCODE_BOOTSTRAP__?: {
      token?: string;
      isDesktopHost?: boolean;
      serverOrigin?: string;
    };
  }
}

export interface HostBridge {
  isDesktopHost: boolean;
  canSelectDirectory: boolean;
  canOpenEditor: boolean;
  canOpenDebugWorkbench: boolean;
  selectDirectory(): Promise<string | null>;
  openConfigInEditor(path?: string): Promise<void>;
  openDebugWorkbench(sessionId?: string | null): Promise<void>;
}

function browserBridge(): HostBridge {
  return {
    isDesktopHost: false,
    canSelectDirectory: false,
    canOpenEditor: false,
    canOpenDebugWorkbench: true,
    selectDirectory() {
      return Promise.resolve(null);
    },
    async openConfigInEditor(path?: string) {
      if (!path) {
        return;
      }
      try {
        await navigator.clipboard.writeText(path);
      } catch {
        // 浏览器降级为静默失败；调用方仍会展示路径。
      }
    },
    async openDebugWorkbench(sessionId?: string | null) {
      const url = new URL('/debug.html', window.location.origin);
      url.searchParams.set('debugWorkbench', '1');
      if (sessionId) {
        url.searchParams.set('sessionId', sessionId);
      }
      window.open(url.toString(), '_blank', 'noopener,noreferrer');
    },
  };
}

function desktopBridge(): HostBridge {
  return {
    isDesktopHost: true,
    canSelectDirectory: true,
    canOpenEditor: true,
    canOpenDebugWorkbench: import.meta.env.DEV,
    async selectDirectory() {
      await waitForTauriEnvironment();
      return invoke<string | null>('select_directory');
    },
    async openConfigInEditor(path?: string) {
      await waitForTauriEnvironment();
      await invoke('open_config_in_editor', { path: path ?? null });
    },
    async openDebugWorkbench(sessionId?: string | null) {
      await waitForTauriEnvironment();
      await invoke('open_debug_workbench', { sessionId: sessionId ?? null });
    },
  };
}

export function getHostBridge(): HostBridge {
  const injectedDesktopFlag = Boolean(window.__ASTRCODE_BOOTSTRAP__?.isDesktopHost);
  if (isTauriEnvironment() || injectedDesktopFlag) {
    return desktopBridge();
  }
  return browserBridge();
}
