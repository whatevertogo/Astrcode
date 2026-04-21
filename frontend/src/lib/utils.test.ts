import { describe, expect, it } from 'vitest';

import { calculateCacheHitRatePercent } from './utils';

describe('calculateCacheHitRatePercent', () => {
  it('uses total input as the denominator instead of uncached provider input only', () => {
    expect(
      calculateCacheHitRatePercent({
        providerCacheMetricsSupported: true,
        providerInputTokens: 4_740,
        cacheReadInputTokens: 54_272,
        cacheCreationInputTokens: 0,
      })
    ).toBe(92);
  });

  it('returns null when the provider does not report cache metrics', () => {
    expect(
      calculateCacheHitRatePercent({
        providerCacheMetricsSupported: false,
        providerInputTokens: 100,
        cacheReadInputTokens: 50,
        cacheCreationInputTokens: 0,
      })
    ).toBeNull();
  });
});
