import React, { memo, useEffect, useState } from 'react';
import type { ToolCallMessage, ToolStatus } from '../../types';
import styles from './ToolCallBlock.module.css';

const STATUS_ICON: Record<ToolStatus, string> = {
  running: '⟳',
  ok:      '✓',
  fail:    '✗',
};

const STATUS_COLOR: Record<ToolStatus, string> = {
  running: '#9cdcfe',
  ok:      '#4ec9b0',
  fail:    '#f44747',
};

interface ToolCallBlockProps {
  message: ToolCallMessage;
}

function ToolCallBlock({ message }: ToolCallBlockProps) {
  const [expanded, setExpanded] = useState(false);

  const borderColor = STATUS_COLOR[message.status];
  const icon = STATUS_ICON[message.status];
  const toolCallId = message.toolCallId ?? 'unknown';
  const toolName = message.toolName ?? '(unknown tool)';
  const shortId = toolCallId.slice(-6);
  const duration = message.durationMs != null ? `${message.durationMs}ms` : '';
  const preview = message.error ?? message.output ?? (message.status === 'running' ? '执行中...' : '');

  useEffect(() => {
    if (message.status === 'fail') {
      setExpanded(true);
    }
  }, [message.status]);

  return (
    <div
      className={styles.block}
      style={{ borderLeftColor: borderColor }}
    >
      {/* Header — click to toggle */}
      <div
        className={styles.header}
        onClick={() => setExpanded((v) => !v)}
      >
        <span
          className={`${styles.icon} ${message.status === 'running' ? styles.spinning : ''}`}
          style={{ color: borderColor }}
        >
          {icon}
        </span>
        <span className={styles.toolName}>{toolName}</span>
        <span className={styles.callId}>#{shortId}</span>
        {duration && <span className={styles.duration}>{duration}</span>}
        <span className={styles.chevron}>{expanded ? '▾' : '▸'}</span>
      </div>

      {!expanded && preview && (
        <div className={styles.preview}>{preview}</div>
      )}

      {/* Body */}
      {expanded && (
        <div className={styles.body}>
          {message.output && (
            <pre className={styles.output}>{message.output}</pre>
          )}
          {message.error && (
            <div className={styles.error}>{message.error}</div>
          )}
          {!message.output && !message.error && message.status === 'running' && (
            <div className={styles.running}>执行中...</div>
          )}
        </div>
      )}
    </div>
  );
}

export default memo(ToolCallBlock);
