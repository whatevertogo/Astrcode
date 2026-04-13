//! # Configuration API Endpoints
//!
//! Provider, model, and active selection management.

import type { ConfigView } from '../../types';
import { request, requestJson } from './client';

export interface ConfigReloadResult {
  reloadedAt: string;
  config: ConfigView;
}

export async function getConfig(): Promise<ConfigView> {
  return requestJson<ConfigView>('/api/config');
}

export async function reloadConfig(): Promise<ConfigReloadResult> {
  return requestJson<ConfigReloadResult>('/api/config/reload', {
    method: 'POST',
  });
}

export async function saveActiveSelection(
  activeProfile: string,
  activeModel: string
): Promise<void> {
  await request('/api/config/active-selection', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ activeProfile, activeModel }),
  });
}
