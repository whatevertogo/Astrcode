import { memo } from 'react';

import type { PromptMetricsMessage as PromptMetricsMessageType } from '../../types';

interface PromptMetricsMessageProps {
  message: PromptMetricsMessageType;
}

function formatTokenCount(value?: number): string {
  if (value === undefined) {
    return '—';
  }
  return value.toLocaleString();
}

function calculateCacheHitRatePercent(message: PromptMetricsMessageType): number | null {
  if (!message.providerInputTokens || message.providerInputTokens <= 0) {
    return null;
  }

  const rawRate = Math.round(
    ((message.cacheReadInputTokens ?? 0) / message.providerInputTokens) * 100
  );
  return Math.min(Math.max(rawRate, 0), 100);
}

function PromptMetricsMessage({ message }: PromptMetricsMessageProps) {
  const hitRate = calculateCacheHitRatePercent(message);

  return (
    <div className="ml-[var(--chat-assistant-content-offset)] border border-[rgba(89,132,255,0.2)] bg-[linear-gradient(180deg,rgba(247,249,255,0.98)_0%,rgba(240,244,255,0.96)_100%)] rounded-[18px] px-4 py-3.5 shadow-[0_12px_28px_rgba(68,102,193,0.08)]">
      <div className="flex items-center gap-2.5 flex-wrap mb-3">
        <span className="inline-flex items-center min-h-[26px] px-2.5 rounded-full bg-[rgba(89,132,255,0.14)] text-[#3558c4] text-xs font-bold">
          Prompt 指标
        </span>
        <span className="text-text-muted text-xs">step #{message.stepIndex}</span>
      </div>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(160px,1fr))] gap-3">
        <div>
          <div className="text-text-muted text-xs mb-1">估算上下文</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.estimatedTokens)}
          </div>
        </div>
        <div>
          <div className="text-text-muted text-xs mb-1">有效窗口</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.effectiveWindow)} / {formatTokenCount(message.contextWindow)}
          </div>
        </div>
        <div>
          <div className="text-text-muted text-xs mb-1">Provider 输入 / 输出</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.providerInputTokens)} /{' '}
            {formatTokenCount(message.providerOutputTokens)}
          </div>
        </div>
        <div>
          <div className="text-text-muted text-xs mb-1">Cache 读 / 写</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.cacheReadInputTokens)} /{' '}
            {formatTokenCount(message.cacheCreationInputTokens)}
          </div>
        </div>
      </div>
      <div className="flex gap-3 flex-wrap mt-3 text-text-muted text-xs">
        <span>压缩阈值 {formatTokenCount(message.thresholdTokens)}</span>
        <span>截断工具结果 {message.truncatedToolResults}</span>
        {hitRate === null ? null : <span>缓存命中 {hitRate}% </span>}
      </div>
    </div>
  );
}

export default memo(PromptMetricsMessage);
