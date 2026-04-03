import { memo, useEffect, useState } from 'react';
import type { ToolCallMessage } from '../../types';
import { classifyToolDiffLine, extractToolDiffMetadata } from '../../lib/toolDiff';
import { extractToolShellDisplay } from '../../lib/toolDisplay';
import styles from './ToolCallBlock.module.css';

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
  const shell = extractToolShellDisplay(message.metadata);
  const [userInteracted, setUserInteracted] = useState(false);

  const toolCallId = message.toolCallId ?? 'unknown';
  const toolName = message.toolName ?? '(unknown tool)';

  // 仅在用户未交互且工具状态变为终态时自动展开一次
  useEffect(() => {
    if (
      !userInteracted &&
      (message.status === 'fail' || message.status === 'ok') &&
      (message.output || message.error || diff)
    ) {
      // 自动展开原生 details 元素
      const detailsEl = document.getElementById(toolCallId) as HTMLDetailsElement | null;
      if (detailsEl) detailsEl.open = true;
    }
  }, [diff, message.error, message.output, message.status, userInteracted, toolCallId]);

  // 构造类似 Codex 的摘要文本
  const summaryText = (() => {
    const prefix = message.status === 'running' ? '运行中' : '已运行';
    if (shell?.command) {
      const cmd = shell.command.split('\n')[0].trim();
      return `${prefix} ${toolName} | ${cmd.length > 50 ? `${cmd.slice(0, 50)}...` : cmd}`;
    }
    // 提取 args 中的关键参数
    if (message.args && typeof message.args === 'object' && !Array.isArray(message.args)) {
      const args = message.args as Record<string, unknown>;
      if (args.path) return `${prefix} ${toolName} (${args.path as string})`;
      const firstKey = Object.keys(args)[0];
      if (firstKey)
        return `${prefix} ${toolName} (${String(firstKey)}=${String(args[firstKey]).slice(0, 20)})`;
    }
    return `${prefix} ${toolName}`;
  })();

  return (
    <details
      id={toolCallId}
      className={styles.wrapper}
      onToggle={(e) => {
        if ((e.target as HTMLDetailsElement).open) setUserInteracted(true);
      }}
      open={
        !userInteracted &&
        (message.status === 'fail' || message.status === 'ok') &&
        !!(message.output || message.error || diff)
      }
    >
      <summary className={styles.summary}>
        <span className={styles.summaryText}>{summaryText}</span>
        <span className={styles.summaryChevron}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="9 18 15 12 9 6"></polyline>
          </svg>
        </span>
      </summary>
      <div className={styles.body}>
        {shell && (
          <div className={styles.shellMeta}>
            {shell.command && <div className={styles.shellCommand}>$ {shell.command}</div>}
            <div className={styles.shellMetaRow}>
              {shell.cwd && <span className={styles.shellPill}>{shell.cwd}</span>}
              {shell.shell && <span className={styles.shellPill}>{shell.shell}</span>}
              {typeof shell.exitCode === 'number' && (
                <span className={styles.shellPill}>exit {shell.exitCode}</span>
              )}
            </div>
          </div>
        )}
        {/* 有 diff 时以摘要样式展示 output，无 diff 时以等宽 pre 展示 */}
        {message.output && diff && <div className={styles.outputSummary}>{message.output}</div>}
        {diff && (
          <div className={styles.diffMeta}>
            {diff.changeType && <span className={styles.diffPill}>{diff.changeType}</span>}
            {diff.path && <span className={styles.diffPath}>{diff.path}</span>}
            {typeof diff.bytes === 'number' && (
              <span className={styles.diffPill}>{diff.bytes} bytes</span>
            )}
          </div>
        )}
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
        {shell && (
          <div className={styles.terminal}>
            {shell.segments.length > 0 ? (
              shell.segments.map((segment, index) => (
                <div
                  key={`${toolCallId}-segment-${index}`}
                  className={
                    segment.stream === 'stderr'
                      ? `${styles.terminalSegment} ${styles.terminalSegmentError}`
                      : styles.terminalSegment
                  }
                >
                  {segment.text}
                </div>
              ))
            ) : message.output ? (
              <pre className={styles.output}>{message.output}</pre>
            ) : (
              <div className={styles.running}>执行中...</div>
            )}
          </div>
        )}
        {message.output && !diff && !shell && <pre className={styles.output}>{message.output}</pre>}
        {message.error && <div className={styles.error}>{message.error}</div>}
        {diff?.truncated && (
          <div className={styles.note}>diff 已截断，完整变更请直接查看文件。</div>
        )}
        {!message.output && !message.error && !shell && message.status === 'running' && (
          <div className={styles.running}>执行中...</div>
        )}
      </div>
    </details>
  );
}

export default memo(ToolCallBlock);
