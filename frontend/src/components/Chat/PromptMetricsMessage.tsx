import { memo } from 'react';

import type { PromptMetricsMessage as PromptMetricsMessageType } from '../../types';
import styles from './PromptMetricsMessage.module.css';

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
  const hitRate =
    message.providerInputTokens && message.providerInputTokens > 0
      ? Math.round(((message.cacheReadInputTokens ?? 0) / message.providerInputTokens) * 100)
      : null;

  return (
    <div className={styles.wrapper}>
      <div className={styles.header}>
        <span className={styles.badge}>Prompt 指标</span>
        <span className={styles.meta}>step #{message.stepIndex}</span>
      </div>
      <div className={styles.grid}>
        <div>
          <div className={styles.label}>估算上下文</div>
          <div className={styles.value}>{formatTokenCount(message.estimatedTokens)}</div>
        </div>
        <div>
          <div className={styles.label}>有效窗口</div>
          <div className={styles.value}>
            {formatTokenCount(message.effectiveWindow)} / {formatTokenCount(message.contextWindow)}
          </div>
        </div>
        <div>
          <div className={styles.label}>Provider 输入 / 输出</div>
          <div className={styles.value}>
            {formatTokenCount(message.providerInputTokens)} /{' '}
            {formatTokenCount(message.providerOutputTokens)}
          </div>
        </div>
        <div>
          <div className={styles.label}>Cache 读 / 写</div>
          <div className={styles.value}>
            {formatTokenCount(message.cacheReadInputTokens)} /{' '}
            {formatTokenCount(message.cacheCreationInputTokens)}
          </div>
        </div>
      </div>
      <div className={styles.footer}>
        <span>压缩阈值 {formatTokenCount(message.thresholdTokens)}</span>
        <span>截断工具结果 {message.truncatedToolResults}</span>
        {hitRate === null ? null : <span>缓存命中 {hitRate}% </span>}
      </div>
    </div>
  );
}

export default memo(PromptMetricsMessage);
