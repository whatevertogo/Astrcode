//! # Composer API Endpoints
//!
//! Session-scoped composer option queries for the input bar.

import type { ComposerOption } from '../../types';
import { requestJson } from './client';

interface ComposerOptionsResponse {
  items: ComposerOption[];
}

export async function listComposerOptions(
  sessionId: string,
  query: string,
  signal?: AbortSignal
): Promise<ComposerOption[]> {
  const params = new URLSearchParams();
  params.set('kinds', 'skill');
  if (query.trim()) {
    params.set('q', query.trim());
  }

  const response = await requestJson<ComposerOptionsResponse>(
    `/api/sessions/${encodeURIComponent(sessionId)}/composer/options?${params.toString()}`,
    { signal }
  );
  return response.items;
}
