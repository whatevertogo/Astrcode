//! # Configuration API Endpoints
//!
//! Provider, model, and active selection management.

import type { ConfigView } from '../../types';
import { request, requestJson } from './client';

export async function getConfig(): Promise<ConfigView> {
  return requestJson<ConfigView>('/api/config');
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
