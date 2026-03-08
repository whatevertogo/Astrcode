import { isTauri as coreIsTauri } from '@tauri-apps/api/core';

const TAURI_UNAVAILABLE_MESSAGE =
  'Tauri IPC 不可用。当前运行在浏览器调试模式；如需桌面能力，请使用 cargo tauri dev 启动桌面应用。';
const TAURI_WAIT_TIMEOUT_MS = 8000;
const TAURI_WAIT_INTERVAL_MS = 50;

function hasTauriInternals(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }

  const maybeWindow = window as typeof window & {
    __TAURI_INTERNALS__?: {
      invoke?: unknown;
      transformCallback?: unknown;
    };
  };

  const internals = maybeWindow.__TAURI_INTERNALS__;
  return typeof internals?.invoke === 'function';
}

function hasTauriFlag(): boolean {
  try {
    return coreIsTauri();
  } catch {
    return false;
  }
}

export function isTauriEnvironment(): boolean {
  return hasTauriFlag() || hasTauriInternals();
}

export function assertTauriEnvironment(): void {
  if (!hasTauriInternals()) {
    throw new Error(TAURI_UNAVAILABLE_MESSAGE);
  }
}

export function getTauriUnavailableMessage(): string {
  return TAURI_UNAVAILABLE_MESSAGE;
}

export async function tryWaitForTauriEnvironment(
  timeoutMs = TAURI_WAIT_TIMEOUT_MS,
): Promise<boolean> {
  const startedAt = Date.now();

  // invoke() requires internals; only waiting on flag can race at startup.
  const checkTauri = () => hasTauriInternals();

  while (!checkTauri()) {
    if (Date.now() - startedAt >= timeoutMs) {
      console.log('[tryWaitForTauriEnvironment] Timeout after', timeoutMs, 'ms');
      return false;
    }

    await new Promise((resolve) => window.setTimeout(resolve, TAURI_WAIT_INTERVAL_MS));
  }

  return true;
}

export async function waitForTauriEnvironment(
  timeoutMs = TAURI_WAIT_TIMEOUT_MS,
): Promise<void> {
  if (!(await tryWaitForTauriEnvironment(timeoutMs))) {
    throw new Error(TAURI_UNAVAILABLE_MESSAGE);
  }
}
