import { isTauriEnvironment } from './tauri';

declare global {
  interface Window {
    __ASTRCODE_BOOTSTRAP__?: {
      token?: string;
      isDesktopHost?: boolean;
      serverOrigin?: string;
    };
  }
}

let bootstrapToken: string | null | undefined;
let browserServerOrigin: string | null = null;
let sessionToken: string | null | undefined;
let sessionTokenExpiresAtMs = 0;
let bootstrapSessionReady: Promise<void> | null = null;
const BOOTSTRAP_WAIT_TIMEOUT_MS = 8000;
const BOOTSTRAP_WAIT_INTERVAL_MS = 50;
const BROWSER_BOOTSTRAP_PATH = '/__astrcode__/run-info';
const LOCAL_DEV_PORT = '5173';

interface BrowserBootstrapPayload {
  token?: string;
  serverOrigin?: string;
}

interface AuthExchangeResponse {
  ok?: boolean;
  token?: string;
  expiresAtMs?: number;
}

export function getServerOrigin(): string {
  const injected = window.__ASTRCODE_BOOTSTRAP__?.serverOrigin?.trim();
  if (injected) {
    return injected.replace(/\/+$/, '');
  }
  if (browserServerOrigin) {
    return browserServerOrigin;
  }
  return window.location.origin.replace(/\/+$/, '');
}

export function getServerAuthToken(): string | null {
  if (sessionToken && Date.now() < sessionTokenExpiresAtMs) {
    return sessionToken;
  }
  return null;
}

function getBootstrapToken(): string | null {
  const injected = window.__ASTRCODE_BOOTSTRAP__?.token;
  if (typeof injected === 'string' && injected.trim()) {
    return injected.trim();
  }
  return null;
}

function cacheBootstrapToken(token: string): void {
  bootstrapToken = token;
}

function cacheServerSession(token: string, expiresAtMs: number): void {
  sessionToken = token;
  sessionTokenExpiresAtMs = expiresAtMs;
}

function hasDesktopBootstrap(): boolean {
  const token = window.__ASTRCODE_BOOTSTRAP__?.token?.trim();
  const serverOrigin = window.__ASTRCODE_BOOTSTRAP__?.serverOrigin?.trim();
  return Boolean(token && serverOrigin);
}

function isTauriWindowOrigin(): boolean {
  const { protocol, hostname } = window.location;
  return (
    protocol === 'tauri:' || hostname === 'tauri.localhost' || hostname.endsWith('.tauri.localhost')
  );
}

function shouldWaitForDesktopBootstrap(): boolean {
  return isTauriEnvironment() || isTauriWindowOrigin();
}

function shouldUseBrowserBootstrapBridge(): boolean {
  const { protocol, hostname, port } = window.location;
  return (
    (protocol === 'http:' || protocol === 'https:') &&
    (hostname === '127.0.0.1' || hostname === 'localhost') &&
    port === LOCAL_DEV_PORT
  );
}

async function waitForDesktopBootstrap(): Promise<void> {
  if (typeof window === 'undefined' || !shouldWaitForDesktopBootstrap() || hasDesktopBootstrap()) {
    return;
  }

  const startedAt = Date.now();
  while (!hasDesktopBootstrap()) {
    if (Date.now() - startedAt >= BOOTSTRAP_WAIT_TIMEOUT_MS) {
      throw new Error('desktop bootstrap was not injected before the first API request');
    }
    await new Promise((resolve) => window.setTimeout(resolve, BOOTSTRAP_WAIT_INTERVAL_MS));
  }
}

async function hydrateBrowserBootstrap(): Promise<void> {
  if (typeof window === 'undefined' || !shouldUseBrowserBootstrapBridge() || bootstrapToken) {
    return;
  }

  const response = await fetch(BROWSER_BOOTSTRAP_PATH, {
    cache: 'no-store',
  });
  if (!response.ok) {
    throw new Error('浏览器前端尚未获取到本地服务 bootstrap 信息，请确认 astrcode-server 已启动。');
  }

  const payload = (await response.json()) as BrowserBootstrapPayload;
  const token = payload.token?.trim();
  if (!token) {
    throw new Error('浏览器 bootstrap 返回的数据不完整（缺少 token）。');
  }

  const origin = payload.serverOrigin?.trim()?.replace(/\/+$/, '');
  if (origin) {
    browserServerOrigin = origin;
  }

  cacheBootstrapToken(token);
}

async function exchangeBootstrapToken(token: string): Promise<void> {
  const response = await fetch(`${getServerOrigin()}/api/auth/exchange`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ token }),
  });
  if (!response.ok) {
    throw new Error(
      '本地服务拒绝了 bootstrap 凭据，请确认 astrcode-server 仍然是当前会话启动的实例。'
    );
  }

  const payload = (await response.json()) as AuthExchangeResponse;
  const sessionTokenValue = payload.token?.trim();
  if (!payload.ok || !sessionTokenValue || typeof payload.expiresAtMs !== 'number') {
    throw new Error('本地服务返回的鉴权交换结果不完整。');
  }

  cacheServerSession(sessionTokenValue, payload.expiresAtMs);
}

function clearTokenFromUrl(): void {
  const url = new URL(window.location.href);
  if (!url.searchParams.has('token')) {
    return;
  }
  url.searchParams.delete('token');
  window.history.replaceState({}, document.title, `${url.pathname}${url.search}${url.hash}`);
}

export function ensureServerSession(): Promise<void> {
  if (!bootstrapSessionReady) {
    bootstrapSessionReady = (async () => {
      await waitForDesktopBootstrap();
      if (!bootstrapToken) {
        bootstrapToken = getBootstrapToken();
        if (bootstrapToken) {
          clearTokenFromUrl();
        }
      }
      if (!bootstrapToken) {
        await hydrateBrowserBootstrap();
      }
      if (!bootstrapToken) {
        throw new Error('未找到可用于交换的 bootstrap 凭据。');
      }
      if (!sessionToken || Date.now() >= sessionTokenExpiresAtMs) {
        await exchangeBootstrapToken(bootstrapToken);
      }
    })().finally(() => {
      bootstrapSessionReady = null;
    });
  }

  return bootstrapSessionReady.then(() => {
    if (!sessionToken) {
      throw new Error('本地服务鉴权会话尚未建立。');
    }
  });
}
