import { memo, useEffect, useState } from 'react';
import type { ToolCallMessage } from '../../types';
import { classifyToolDiffLine, extractToolDiffMetadata } from '../../lib/toolDiff';
import {
  extractStructuredArgs,
  extractStructuredJsonOutput,
  formatToolCallSummary,
  extractToolMetadataSummary,
  extractToolShellDisplay,
} from '../../lib/toolDisplay';
import { cn } from '../../lib/utils';
import { chevronIcon, pillNeutral, terminalBlock, patchLine } from '../../lib/styles';
import ToolJsonView from './ToolJsonView';

interface ToolCallBlockProps {
  message: ToolCallMessage;
}

function patchLineVariant(line: string): string {
  switch (classifyToolDiffLine(line)) {
    case 'meta':
      return 'bg-surface-muted text-text-secondary';
    case 'header':
      return 'bg-surface-soft text-text-secondary';
    case 'add':
      return 'bg-success-soft text-success';
    case 'remove':
      return 'bg-danger-soft text-danger';
    case 'note':
      return 'bg-warning-soft text-warning';
    case 'context':
    default:
      return 'text-text-secondary';
  }
}

function ToolCallBlock({ message }: ToolCallBlockProps) {
  const diff = extractToolDiffMetadata(message.metadata);
  const shell = extractToolShellDisplay(message.metadata);
  const metadataSummary = extractToolMetadataSummary(message.metadata);
  const structuredArgs = extractStructuredArgs(message.args);
  const structuredJsonOutput = extractStructuredJsonOutput(message.output);
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

  const summaryText = formatToolCallSummary(
    toolName,
    message.args,
    message.status,
    message.metadata
  );

  return (
    <details
      id={toolCallId}
      className="block mb-1 ml-[var(--chat-assistant-content-offset)] animate-block-enter motion-reduce:animate-none group"
      onToggle={(e) => {
        if ((e.target as HTMLDetailsElement).open) setUserInteracted(true);
      }}
      open={
        !userInteracted &&
        (message.status === 'fail' || message.status === 'ok') &&
        !!(message.output || message.error || diff)
      }
    >
      <summary
        className="flex items-center gap-1.5 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none rounded-0 text-text-secondary transition-opacity duration-150 ease-out text-[13px] font-normal font-mono list-none flex-nowrap w-full min-w-0 [&::-webkit-details-marker]:hidden hover:opacity-70"
        title={summaryText}
      >
        <span className="block flex-1 whitespace-nowrap overflow-hidden text-ellipsis min-w-0">
          {summaryText}
        </span>
        <span className={chevronIcon}>
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
      <div className="mt-1 mb-2">
        {structuredArgs && (
          <div className="pb-3">
            <div className="mb-2 text-xs leading-snug text-text-secondary">调用参数</div>
            <ToolJsonView value={structuredArgs.value} summary={structuredArgs.summary} />
          </div>
        )}
        {shell && (
          <div className="pb-3">
            {shell.command && (
              <div className="font-mono text-[13px] leading-relaxed text-text-primary overflow-wrap-anywhere">
                $ {shell.command}
              </div>
            )}
            <div className="mt-2 flex flex-wrap items-center gap-2">
              {shell.cwd && <span className={pillNeutral}>{shell.cwd}</span>}
              {shell.shell && <span className={pillNeutral}>{shell.shell}</span>}
              {typeof shell.exitCode === 'number' && (
                <span className={pillNeutral}>exit {shell.exitCode}</span>
              )}
            </div>
          </div>
        )}
        {/* 有 diff 时以摘要样式展示 output，无 diff 时以等宽 pre 展示 */}
        {message.output && diff && (
          <div className="pb-3 text-[13px] text-text-secondary leading-relaxed">
            {message.output}
          </div>
        )}
        {diff && (
          <div className="flex flex-wrap items-center gap-2 pb-3">
            {diff.changeType && <span className={pillNeutral}>{diff.changeType}</span>}
            {diff.path && (
              <span className="text-xs text-text-secondary font-mono overflow-wrap-anywhere">
                {diff.path}
              </span>
            )}
            {typeof diff.bytes === 'number' && (
              <span className={pillNeutral}>{diff.bytes} bytes</span>
            )}
          </div>
        )}
        {!diff && metadataSummary?.pills.length ? (
          <div className="flex items-center flex-wrap gap-2">
            {metadataSummary.pills.map((pill, index) => (
              <span key={`${toolCallId}-${index}-${pill}`} className={pillNeutral}>
                {pill}
              </span>
            ))}
          </div>
        ) : null}
        {diff && (
          <div className={terminalBlock}>
            {diff.patch.split('\n').map((line, index) => (
              <div key={`${toolCallId}-${index}`} className={cn(patchLine, patchLineVariant(line))}>
                {line || ' '}
              </div>
            ))}
          </div>
        )}
        {shell && (
          <div className={terminalBlock}>
            {shell.segments.length > 0 ? (
              shell.segments.map((segment, index) => (
                <div
                  key={`${toolCallId}-segment-${index}`}
                  className={cn(
                    'px-4 py-2 font-mono text-[13px] leading-relaxed whitespace-pre-wrap overflow-wrap-anywhere text-text-primary',
                    segment.stream === 'stderr' && 'text-terminal-error'
                  )}
                >
                  {segment.text}
                </div>
              ))
            ) : message.output ? (
              <pre className="m-0 px-4 py-3 font-mono text-[13px] text-text-primary max-h-[320px] overflow-y-auto whitespace-pre-wrap overflow-wrap-anywhere leading-relaxed bg-surface border border-border rounded-lg">
                {message.output}
              </pre>
            ) : (
              <div className="px-[18px] pt-3.5 pb-[18px] text-[13px] text-text-secondary">
                执行中...
              </div>
            )}
          </div>
        )}
        {message.output && !diff && !shell && structuredJsonOutput ? (
          <ToolJsonView value={structuredJsonOutput.value} summary={structuredJsonOutput.summary} />
        ) : null}
        {message.output && !diff && !shell && !structuredJsonOutput && (
          <pre className="m-0 px-4 py-3 font-mono text-[13px] text-text-primary max-h-[320px] overflow-y-auto whitespace-pre-wrap overflow-wrap-anywhere leading-relaxed bg-surface border border-border rounded-lg">
            {message.output}
          </pre>
        )}
        {message.error && (
          <div className="px-3.5 py-3.5 text-[13px] text-danger font-mono">{message.error}</div>
        )}
        {!message.error && metadataSummary?.message && (
          <div className="px-[18px] py-3 pb-[18px] text-xs text-text-secondary">
            {metadataSummary.message}
          </div>
        )}
        {diff?.truncated && (
          <div className="px-[18px] py-3 pb-[18px] text-xs text-text-secondary">
            diff 已截断，完整变更请直接查看文件。
          </div>
        )}
        {!message.output && !message.error && !shell && message.status === 'running' && (
          <div className="px-[18px] pt-3.5 pb-[18px] text-[13px] text-text-secondary">
            执行中...
          </div>
        )}
      </div>
    </details>
  );
}

export default memo(ToolCallBlock);
