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
  selectDirectory(): Promise<string | null>;
  openConfigInEditor(path?: string): Promise<void>;
}

function browserBridge(): HostBridge {
  return {
    isDesktopHost: false,
    canSelectDirectory: false,
    canOpenEditor: false,
    async selectDirectory() {
      return null;
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
  };
}

function desktopBridge(): HostBridge {
  return {
    isDesktopHost: true,
    canSelectDirectory: true,
    canOpenEditor: true,
    async selectDirectory() {
      await waitForTauriEnvironment();
      return invoke<string | null>('select_directory');
    },
    async openConfigInEditor(path?: string) {
      await waitForTauriEnvironment();
      await invoke('open_config_in_editor', { path: path ?? null });
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
