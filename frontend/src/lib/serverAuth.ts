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

export function getServerOrigin(): string {
  const injected = window.__ASTRCODE_BOOTSTRAP__?.serverOrigin?.trim();
  if (injected) {
    return injected.replace(/\/+$/, '');
  }
  return window.location.origin;
}

export function getServerAuthToken(): string | null {
  if (bootstrapToken !== undefined) {
    return bootstrapToken;
  }

  bootstrapToken = getBootstrapToken();
  if (bootstrapToken) {
    clearTokenFromUrl();
  }
  return bootstrapToken;
}

function getBootstrapToken(): string | null {
  const injected = window.__ASTRCODE_BOOTSTRAP__?.token;
  if (typeof injected === 'string' && injected.trim()) {
    return injected.trim();
  }
  return null;
}

function clearTokenFromUrl(): void {
  const url = new URL(window.location.href);
  if (!url.searchParams.has('token')) {
    return;
  }
  url.searchParams.delete('token');
  window.history.replaceState({}, document.title, `${url.pathname}${url.search}${url.hash}`);
}

export async function ensureServerSession(): Promise<void> {
  getServerAuthToken();
}
