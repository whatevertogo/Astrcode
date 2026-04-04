//! # API Client
//!
//! HTTP request utilities, auth header injection, and error normalization.
//!
//! This module encapsulates all low-level fetch logic so higher-level
//! services (sessions, config, models) can focus on their domain paths
//! without worrying about token exchange or CORS.

import { ensureServerSession, getServerAuthToken, getServerOrigin } from '../serverAuth';

function buildAuthHeaders(headers?: HeadersInit): Headers {
  const merged = new Headers(headers);
  const token = getServerAuthToken();
  if (token) {
    merged.set('x-astrcode-token', token);
  }
  return merged;
}

export async function getErrorMessage(response: Response): Promise<string> {
  let message = `${response.status} ${response.statusText}`;
  try {
    const payload = (await response.json()) as { error?: unknown };
    if (typeof payload.error === 'string' && payload.error) {
      message = payload.error;
    }
  } catch {
    // ignore
  }

  return message;
}

export async function ensureOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  const message = await getErrorMessage(response);
  throw new Error(message);
}

export function normalizeFetchError(error: unknown): Error {
  if (error instanceof Error && error.name === 'AbortError') {
    return error;
  }

  if (error instanceof TypeError) {
    if (window.__ASTRCODE_BOOTSTRAP__?.isDesktopHost) {
      return new Error(
        '无法连接本地服务，请确认 AstrCode 桌面端仍在运行；如果刚关闭了启动它的终端，请重新执行 `cargo tauri dev`。'
      );
    }
    return new Error('无法连接后端服务，请确认本地 server 或网络连接正常。');
  }

  return error instanceof Error ? error : new Error(String(error));
}

export async function requestRaw(path: string, init?: RequestInit): Promise<Response> {
  await ensureServerSession();
  try {
    return await fetch(`${getServerOrigin()}${path}`, {
      ...init,
      headers: buildAuthHeaders(init?.headers),
    });
  } catch (error) {
    throw normalizeFetchError(error);
  }
}

export async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await requestRaw(path, init);
  await ensureOk(response);
  return (await response.json()) as T;
}

export async function request(path: string, init?: RequestInit): Promise<Response> {
  const response = await requestRaw(path, init);
  await ensureOk(response);
  return response;
}
