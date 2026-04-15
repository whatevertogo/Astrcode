import { memo } from 'react';

import type { ToolCallMessage } from '../../types';
import { pillDanger, pillNeutral, pillSuccess } from '../../lib/styles';
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
  return (
    <div className="mb-1 ml-[var(--chat-assistant-content-offset)] min-w-0 max-w-full animate-block-enter motion-reduce:animate-none">
      <div className="flex min-w-0 items-center gap-2 font-mono text-[13px] leading-relaxed text-text-secondary">
        <span className={cn('shrink-0', statusPill(message.status))}>{message.toolName}</span>
        <span className="min-w-0 truncate text-text-primary">{message.output ?? '调用工具'}</span>
        <span className="shrink-0 text-text-muted">{statusLabel(message.status)}</span>
      </div>
      {message.error ? (
        <div className="mt-1.5 font-mono text-[12px] leading-relaxed text-danger">
          {message.error}
        </div>
      ) : null}
    </div>
  );
}

export default memo(ToolCallBlock);
