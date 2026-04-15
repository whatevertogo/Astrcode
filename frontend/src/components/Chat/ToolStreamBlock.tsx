import { memo } from 'react';

import type { ToolStreamMessage } from '../../types';
import { extractStructuredJsonOutput } from '../../lib/toolDisplay';
import { pillDanger, pillNeutral, pillSuccess } from '../../lib/styles';
import ToolJsonView from './ToolJsonView';
import ToolCodePanel from './ToolCodePanel';

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
  const structuredResult =
    message.stream === 'stdout' ? extractStructuredJsonOutput(message.content) : null;

  return (
    <div className="mb-2 ml-[var(--chat-assistant-content-offset)] min-w-0 max-w-full animate-block-enter motion-reduce:animate-none">
      <div className="mb-2 flex items-center gap-2 font-mono text-[12px] text-text-secondary">
        <span className={statusPill(message.status)}>
          {message.stream === 'stdout' ? '结果' : streamLabel(message.stream)}
        </span>
        <span className="text-text-muted">{statusLabel(message.status)}</span>
      </div>
      {structuredResult ? (
        <ToolJsonView
          value={structuredResult.value}
          summary={structuredResult.summary}
          defaultOpen={true}
        />
      ) : (
        <ToolCodePanel
          title={message.stream === 'stderr' ? 'Error output' : 'Result'}
          tone={message.stream === 'stderr' ? 'error' : 'normal'}
          content={message.content}
        />
      )}
    </div>
  );
}

export default memo(ToolStreamBlock);
