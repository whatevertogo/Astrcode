import { memo } from 'react';

import type { ToolStreamMessage } from '../../types';
import { pillDanger, pillNeutral, pillSuccess, terminalBlock } from '../../lib/styles';
import { cn } from '../../lib/utils';

interface ToolStreamBlockProps {
  message: ToolStreamMessage;
}

function streamLabel(stream: ToolStreamMessage['stream']): string {
  return stream === 'stderr' ? 'stderr' : 'stdout';
}

function statusPill(status: ToolStreamMessage['status']): string {
  switch (status) {
    case 'ok':
      return pillSuccess;
    case 'fail':
      return pillDanger;
    default:
      return pillNeutral;
  }
}

function statusLabel(status: ToolStreamMessage['status']): string {
  switch (status) {
    case 'ok':
      return 'completed';
    case 'fail':
      return 'failed';
    default:
      return 'running';
  }
}

function ToolStreamBlock({ message }: ToolStreamBlockProps) {
  return (
    <div className="mb-2 ml-[var(--chat-assistant-content-offset)] min-w-0 max-w-full animate-block-enter motion-reduce:animate-none">
      <div className="mb-1.5 flex items-center gap-2 font-mono text-[12px] text-text-secondary">
        <span className={statusPill(message.status)}>{streamLabel(message.stream)}</span>
        <span className="text-text-muted">{statusLabel(message.status)}</span>
      </div>
      <div className={terminalBlock}>
        <pre
          className={cn(
            'm-0 px-4 py-3 font-mono text-[13px] leading-relaxed whitespace-pre-wrap overflow-wrap-anywhere',
            message.stream === 'stderr' ? 'text-terminal-error' : 'text-text-primary'
          )}
        >
          {message.content}
        </pre>
      </div>
    </div>
  );
}

export default memo(ToolStreamBlock);
