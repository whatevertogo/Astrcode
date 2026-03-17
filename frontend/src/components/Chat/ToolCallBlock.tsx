import { memo, useEffect, useState } from 'react';
import type { ToolCallMessage, ToolStatus } from '../../types';
import styles from './ToolCallBlock.module.css';

const STATUS_ICON: Record<ToolStatus, string> = {
  running: '⟳',
  ok: '✓',
  fail: '✕',
};

const STATUS_COLOR: Record<ToolStatus, string> = {
  running: '#7ca9cc',
  ok: '#3ebc7f',
  fail: '#cc5b5b',
};

interface ToolCallBlockProps {
  message: ToolCallMessage;
}

function ToolCallBlock({ message }: ToolCallBlockProps) {
  const [expanded, setExpanded] = useState(Boolean(message.output || message.error));

  const borderColor = STATUS_COLOR[message.status];
  const toolCallId = message.toolCallId ?? 'unknown';
  const toolName = message.toolName ?? '(unknown tool)';
  const shortId = toolCallId.slice(-6);
  const duration = typeof message.durationMs === 'number' ? `${message.durationMs}ms` : '';
  const preview =
    message.error ?? message.output ?? (message.status === 'running' ? '执行中...' : '');

  useEffect(() => {
    if (message.status === 'fail' || message.output || message.error) {
      setExpanded(true);
    }
  }, [message.error, message.output, message.status]);

  return (
    <div className={styles.wrapper}>
      <div className={styles.avatar} aria-hidden="true">
        <svg viewBox="0 0 20 20">
          <rect
            x="3.25"
            y="3.25"
            width="13.5"
            height="13.5"
            rx="3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
          />
          <path
            d="M10 6.4v7.2M6.4 10h7.2"
            fill="none"
            stroke="currentColor"
            strokeLinecap="round"
            strokeWidth="1.4"
          />
        </svg>
      </div>
      <div className={styles.block}>
        <button className={styles.header} type="button" onClick={() => setExpanded((v) => !v)}>
          <span className={styles.headerMain}>
            <span
              className={`${styles.statusIcon} ${message.status === 'running' ? styles.spinning : ''}`}
              style={{
                color: borderColor,
                backgroundColor:
                  message.status === 'ok'
                    ? 'var(--success-soft)'
                    : message.status === 'fail'
                      ? 'var(--danger-soft)'
                      : '#eef5fb',
              }}
            >
              {STATUS_ICON[message.status]}
            </span>
            <span className={styles.toolName}>{toolName}</span>
            <span className={styles.callId}>#{shortId}</span>
          </span>
          <span className={styles.headerMeta}>
            {duration && <span className={styles.duration}>{duration}</span>}
            <span className={styles.toggleLabel}>{expanded ? '收起详情' : '展开详情'}</span>
            <span className={styles.chevron}>{expanded ? '⌃' : '⌄'}</span>
          </span>
        </button>

        {!expanded && preview && <div className={styles.preview}>{preview}</div>}

        {expanded && (
          <div className={styles.body}>
            {message.output && <pre className={styles.output}>{message.output}</pre>}
            {message.error && <div className={styles.error}>{message.error}</div>}
            {!message.output && !message.error && message.status === 'running' && (
              <div className={styles.running}>执行中...</div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(ToolCallBlock);
