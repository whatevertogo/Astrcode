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
  // slash 面板只取会直接以 `/...` 形式插入的 surface；
  // capability 不属于 slash command，因此这里显式限定为 skill + command。
  params.set('kinds', 'skill,command');
  if (query.trim()) {
    params.set('q', query.trim());
  }

  const response = await requestJson<ComposerOptionsResponse>(
    `/api/sessions/${encodeURIComponent(sessionId)}/composer/options?${params.toString()}`,
    { signal }
  );
  return response.items;
}
