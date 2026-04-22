import { memo } from 'react';

import type { PromptMetricsMessage as PromptMetricsMessageType } from '../../types';
import { pillInfo } from '../../lib/styles';
import { calculateCacheHitRatePercent, calculatePromptReuseRatePercent } from '../../lib/utils';

interface PromptMetricsMessageProps {
  message: PromptMetricsMessageType;
}

function formatTokenCount(value?: number): string {
  if (value === undefined) {
    return '—';
  }
  return value.toLocaleString();
}

function formatBreakReason(reason: string): string {
  switch (reason) {
    case 'system_prompt_changed':
      return 'System Prompt 变化';
    case 'tool_schemas_changed':
      return '工具 Schema 变化';
    case 'model_changed':
      return '模型变化';
    case 'global_cache_strategy_changed':
      return '全局缓存策略变化';
    case 'compacted_prompt':
      return '发生 compact';
    case 'tool_result_rebudgeted':
      return '工具结果重预算';
    default:
      return reason;
  }
}

function PromptMetricsMessage({ message }: PromptMetricsMessageProps) {
  const providerHitRate = calculateCacheHitRatePercent(message);
  const promptReuseRate = calculatePromptReuseRatePercent(message);
  const diagnostics = message.promptCacheDiagnostics;
  const unchangedLayers = message.promptCacheUnchangedLayers ?? [];

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
          <div className="text-text-muted text-xs mb-1">KV Cache 读 / 写</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.cacheReadInputTokens)} /{' '}
            {formatTokenCount(message.cacheCreationInputTokens)}
          </div>
        </div>
        <div>
          <div className="text-text-muted text-xs mb-1">Prompt 复用 命中 / 未命中</div>
          <div className="text-text-primary text-sm font-semibold">
            {formatTokenCount(message.promptCacheReuseHits)} /{' '}
            {formatTokenCount(message.promptCacheReuseMisses)}
          </div>
        </div>
      </div>
      <div className="flex gap-3 flex-wrap mt-3 text-text-muted text-xs">
        <span>压缩阈值 {formatTokenCount(message.thresholdTokens)}</span>
        <span>截断工具结果 {message.truncatedToolResults}</span>
        <span>
          Provider Cache{' '}
          {message.providerCacheMetricsSupported
            ? providerHitRate === null
              ? '已启用，当前 step 无读缓存'
              : `命中 ${providerHitRate}%`
            : '未上报'}
        </span>
        {promptReuseRate === null ? null : <span>Prompt 复用 {promptReuseRate}%</span>}
        {unchangedLayers.length === 0 ? null : (
          <span>未变化层 {unchangedLayers.join(' / ')}</span>
        )}
      </div>
      {diagnostics ? (
        <div className="mt-3 flex flex-wrap gap-3 text-xs">
          <span
            className={
              diagnostics.cacheBreakDetected ? 'font-semibold text-danger' : 'text-text-muted'
            }
          >
            {diagnostics.cacheBreakDetected ? '检测到 Cache Break' : '未检测到 Cache Break'}
          </span>
          {diagnostics.expectedDrop ? <span className="text-text-muted">本次跌幅属预期</span> : null}
          {diagnostics.previousCacheReadInputTokens === undefined &&
          diagnostics.currentCacheReadInputTokens === undefined ? null : (
            <span className="text-text-muted">
              读缓存对比 {formatTokenCount(diagnostics.previousCacheReadInputTokens)} →{' '}
              {formatTokenCount(diagnostics.currentCacheReadInputTokens)}
            </span>
          )}
          {diagnostics.reasons.length === 0 ? null : (
            <span className="text-text-muted">
              原因 {diagnostics.reasons.map(formatBreakReason).join(' / ')}
            </span>
          )}
        </div>
      ) : null}
    </div>
  );
}

export default memo(PromptMetricsMessage);
