import { memo, useEffect, useState } from 'react';
import type { ToolCallMessage, ToolStatus } from '../../types';
import { classifyToolDiffLine, extractToolDiffMetadata } from '../../lib/toolDiff';
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

function patchLineClassName(line: string): string {
  switch (classifyToolDiffLine(line)) {
    case 'meta':
      return styles.patchLineMeta;
    case 'header':
      return styles.patchLineHeader;
    case 'add':
      return styles.patchLineAdd;
    case 'remove':
      return styles.patchLineRemove;
    case 'note':
      return styles.patchLineNote;
    case 'context':
    default:
      return styles.patchLineContext;
  }
}

function ToolCallBlock({ message }: ToolCallBlockProps) {
  const diff = extractToolDiffMetadata(message.metadata);
  const [expanded, setExpanded] = useState(false);
  const [userInteracted, setUserInteracted] = useState(false);

  const borderColor = STATUS_COLOR[message.status];
  const toolCallId = message.toolCallId ?? 'unknown';
  const toolName = message.toolName ?? '(unknown tool)';
  const shortId = toolCallId.slice(-6);
  const duration = typeof message.durationMs === 'number' ? `${message.durationMs}ms` : '';
  const preview = diff
    ? `${diff.path ?? toolName}  +${diff.addedLines ?? 0} -${diff.removedLines ?? 0}`
    : (message.error ?? message.output ?? (message.status === 'running' ? '执行中...' : ''));

  // 仅在用户未交互且工具状态变为终态时自动展开一次
  useEffect(() => {
    if (
      !userInteracted &&
      (message.status === 'fail' || message.status === 'ok') &&
      (message.output || message.error || diff)
    ) {
      setExpanded(true);
    }
  }, [diff, message.error, message.output, message.status, userInteracted]);

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
        <button
          className={styles.header}
          type="button"
          onClick={() => {
            setUserInteracted(true);
            setExpanded((v) => !v);
          }}
        >
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
            {/* 有 diff 时以摘要样式展示 output，无 diff 时以等宽 pre 展示 */}
            {message.output && diff && <div className={styles.summary}>{message.output}</div>}
            {diff && (
              <div className={styles.patch}>
                {diff.patch.split('\n').map((line, index) => (
                  <div
                    key={`${toolCallId}-${index}`}
                    className={`${styles.patchLine} ${patchLineClassName(line)}`}
                  >
                    {line || ' '}
                  </div>
                ))}
              </div>
            )}
            {message.output && !diff && <pre className={styles.output}>{message.output}</pre>}
            {message.error && <div className={styles.error}>{message.error}</div>}
            {diff?.truncated && (
              <div className={styles.note}>diff 已截断，完整变更请直接查看文件。</div>
            )}
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
