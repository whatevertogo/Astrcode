import { describe, expect, it } from 'vitest';
import {
  buildGovernanceSparklinePoints,
  formatRatioBps,
  type DebugWorkbenchTrendSample,
} from './debugWorkbench';

describe('formatRatioBps', () => {
  it('formats basis points as percentage', () => {
    expect(formatRatioBps(1234)).toBe('12.34%');
  });

  it('returns fallback for missing values', () => {
    expect(formatRatioBps(undefined)).toBe('—');
  });
});

describe('buildGovernanceSparklinePoints', () => {
  it('projects timeline samples into drawable points', () => {
    const samples: DebugWorkbenchTrendSample[] = [
      { timestamp: 1_000, spawnRejectionRatioBps: 500 },
      { timestamp: 2_000, spawnRejectionRatioBps: 1_000 },
    ];

    const points = buildGovernanceSparklinePoints(
      samples,
      (sample) => sample.spawnRejectionRatioBps,
      120,
      60
    );

    expect(points).toHaveLength(2);
    expect(points[0]?.x).toBeGreaterThanOrEqual(0);
    expect(points[1]?.x).toBeGreaterThan(points[0]?.x ?? 0);
  });
});
