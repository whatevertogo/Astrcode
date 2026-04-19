import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import type { PromptMetricsMessage } from '../types';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * 计算 provider KV cache 命中率百分比（0–100），无有效数据时返回 null。
 */
export function calculateCacheHitRatePercent(
  metrics?: Pick<
    PromptMetricsMessage,
    'providerInputTokens' | 'cacheReadInputTokens' | 'providerCacheMetricsSupported'
  >
): number | null {
  if (!metrics?.providerCacheMetricsSupported) {
    return null;
  }
  if (!metrics?.providerInputTokens || metrics.providerInputTokens <= 0) {
    return null;
  }
  const rawRate = Math.round(
    ((metrics.cacheReadInputTokens ?? 0) / metrics.providerInputTokens) * 100
  );
  return Math.min(Math.max(rawRate, 0), 100);
}

/**
 * 计算 prompt composer 复用命中率百分比（0–100），无有效数据时返回 null。
 */
export function calculatePromptReuseRatePercent(
  metrics?: Pick<PromptMetricsMessage, 'promptCacheReuseHits' | 'promptCacheReuseMisses'>
): number | null {
  const hits = metrics?.promptCacheReuseHits ?? 0;
  const misses = metrics?.promptCacheReuseMisses ?? 0;
  const total = hits + misses;
  if (total <= 0) {
    return null;
  }
  return Math.round((hits / total) * 100);
}
