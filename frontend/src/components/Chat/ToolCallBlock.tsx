import { memo } from 'react';

import type { ToolCallMessage } from '../../types';
import { chevronIcon, pillDanger, pillNeutral, pillSuccess } from '../../lib/styles';
import { cn } from '../../lib/utils';

interface ToolCallBlockProps {
  message: ToolCallMessage;
}

function statusPill(status: ToolCallMessage['status']): string {
  switch (status) {
    case 'ok':
      return pillSuccess;
    case 'fail':
      return pillDanger;
    default:
      return pillNeutral;
  }
}

function statusLabel(status: ToolCallMessage['status']): string {
  switch (status) {
    case 'ok':
      return 'completed';
    case 'fail':
      return 'failed';
    default:
      return 'running';
  }
}

function ToolCallBlock({ message }: ToolCallBlockProps) {
  const summary = message.output?.trim() || '调用工具';
  const bodyText =
    message.error?.trim() ||
    message.output?.trim() ||
    (message.status === 'running' ? '工具已启动，实时输出见下方。' : '工具已完成。');
  const defaultOpen = message.status !== 'running' || Boolean(message.error);

  return (
    <details
      className="group mb-1 ml-[var(--chat-assistant-content-offset)] block min-w-0 max-w-full animate-block-enter motion-reduce:animate-none"
      open={defaultOpen}
    >
      <summary className="flex min-w-0 cursor-pointer items-center gap-2 py-1 font-mono text-[13px] leading-relaxed text-text-secondary list-none [&::-webkit-details-marker]:hidden hover:opacity-80">
        <span className={cn('shrink-0', statusPill(message.status))}>{message.toolName}</span>
        <span className="min-w-0 flex-1 truncate text-text-primary">{summary}</span>
        <span className="shrink-0 text-text-muted">{statusLabel(message.status)}</span>
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
      <div className="mt-1.5 min-w-0 rounded-lg border border-border bg-surface-soft px-3.5 py-3 text-[13px] leading-relaxed text-text-secondary whitespace-pre-wrap overflow-wrap-anywhere">
        {bodyText}
      </div>
    </details>
  );
}

export default memo(ToolCallBlock);
