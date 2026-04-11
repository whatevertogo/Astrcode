import { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { CompactMessage as CompactMessageType } from '../../types';
import { pillSuccess } from '../../lib/styles';

interface CompactMessageProps {
  message: CompactMessageType;
}

function CompactMessage({ message }: CompactMessageProps) {
  const triggerLabel = message.trigger === 'manual' ? '手动压缩' : '自动压缩';

  return (
    <div className="ml-[var(--chat-assistant-content-offset)] rounded-[18px] border border-success/25 bg-success-soft px-4 pb-4 pt-3.5 shadow-soft">
      <div className="mb-3 flex flex-wrap items-center gap-2.5">
        <span className={pillSuccess}>{triggerLabel}</span>
        <span className="text-text-muted text-xs">
          保留最近 {message.preservedRecentTurns} 个 turn
        </span>
      </div>
      <div className="break-words text-sm leading-[1.7] text-text-primary [&_code]:rounded-md [&_code]:bg-black/5 [&_code]:px-[0.35rem] [&_code]:py-[0.1rem] [&_code]:text-[0.92em] [&_ol]:my-[0.4rem] [&_ol]:pl-[1.25rem] [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 [&_ul]:my-[0.4rem] [&_ul]:pl-[1.25rem]">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.summary}</ReactMarkdown>
      </div>
    </div>
  );
}

export default memo(CompactMessage);
