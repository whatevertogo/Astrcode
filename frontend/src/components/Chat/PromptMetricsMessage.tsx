import { memo } from 'react';

import type { PromptMetricsMessage as PromptMetricsMessageType } from '../../types';
import { pillInfo } from '../../lib/styles';
import { calculateCacheHitRatePercent } from '../../lib/utils';

interface PromptMetricsMessageProps {
  message: PromptMetricsMessageType;
}

function formatTokenCount(value?: number): string {
  if (value === undefined) {
    return '—';
  }
  return value.toLocaleString();
}

function PromptMetricsMessage({ message }: PromptMetricsMessageProps) {
  const hitRate = calculateCacheHitRatePercent(message);

  return (
    <div className="ml-[var(--chat-assistant-content-offset)] rounded-[18px] border border-info-border bg-info-soft px-4 py-3.5 shadow-code-panel">
      <div className="mb-3 flex flex-wrap items-center gap-2.5">
        <span className={pillInfo}>Prompt 指标</span>
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
