export interface DebugWorkbenchTrendSample {
  timestamp: number;
  spawnRejectionRatioBps?: number;
  observeToActionRatioBps?: number;
  childReuseRatioBps?: number;
}

export interface SparklinePoint {
  x: number;
  y: number;
}

const GOVERNANCE_TREND_WINDOW_MS = 5 * 60 * 1000;

export function formatRatioBps(value?: number | null): string {
  if (value == null) {
    return '—';
  }
  return `${(value / 100).toFixed(2)}%`;
}

export function buildGovernanceSparklinePoints(
  samples: DebugWorkbenchTrendSample[],
  selector: (sample: DebugWorkbenchTrendSample) => number | undefined,
  width: number,
  height: number,
  padding = 6
): SparklinePoint[] {
  if (samples.length === 0) {
    return [];
  }

  const latestTimestamp = samples[samples.length - 1]?.timestamp ?? 0;
  const windowStart = latestTimestamp - GOVERNANCE_TREND_WINDOW_MS;
  const drawableWidth = Math.max(width - padding * 2, 1);
  const drawableHeight = Math.max(height - padding * 2, 1);

  return samples
    .map((sample) => {
      const value = selector(sample);
      if (value == null) {
        return null;
      }
      const normalizedX =
        Math.min(Math.max(sample.timestamp - windowStart, 0), GOVERNANCE_TREND_WINDOW_MS) /
        GOVERNANCE_TREND_WINDOW_MS;
      const normalizedY = 1 - Math.min(Math.max(value, 0), 10_000) / 10_000;

      return {
        x: padding + normalizedX * drawableWidth,
        y: padding + normalizedY * drawableHeight,
      } satisfies SparklinePoint;
    })
    .filter((point): point is SparklinePoint => point !== null);
}

export function isDebugWorkbenchEnabled(): boolean {
  if (import.meta.env.DEV) {
    return true;
  }
  const query = new URLSearchParams(window.location.search);
  return query.get('debugWorkbench') === '1';
}
