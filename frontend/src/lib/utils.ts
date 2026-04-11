import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import type { PromptMetricsMessage } from '../types';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * 计算 prompt 缓存命中率百分比（0–100），无有效数据时返回 null。
 */
export function calculateCacheHitRatePercent(
  metrics?: Pick<PromptMetricsMessage, 'providerInputTokens' | 'cacheReadInputTokens'>
): number | null {
  if (!metrics?.providerInputTokens || metrics.providerInputTokens <= 0) {
    return null;
  }
  const rawRate = Math.round(
    ((metrics.cacheReadInputTokens ?? 0) / metrics.providerInputTokens) * 100
  );
  return Math.min(Math.max(rawRate, 0), 100);
}
