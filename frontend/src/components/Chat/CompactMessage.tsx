import { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { CompactMessage as CompactMessageType } from '../../types';

interface CompactMessageProps {
  message: CompactMessageType;
}

function CompactMessage({ message }: CompactMessageProps) {
  const triggerLabel = message.trigger === 'manual' ? '手动压缩' : '自动压缩';

  return (
    <div className="ml-[var(--chat-assistant-content-offset)] border border-[rgba(122,185,153,0.28)] bg-[linear-gradient(180deg,rgba(245,252,248,0.98)_0%,rgba(237,247,241,0.96)_100%)] rounded-[18px] px-4 pt-3.5 pb-4 shadow-[0_14px_32px_rgba(63,119,88,0.08)]">
      <div className="flex items-center gap-2.5 flex-wrap mb-3">
        <span className="inline-flex items-center min-h-[26px] px-2.5 rounded-full bg-[rgba(57,201,143,0.14)] text-[#22694c] text-xs font-bold tracking-[0.02em]">
          {triggerLabel}
        </span>
        <span className="text-text-muted text-xs">
          保留最近 {message.preservedRecentTurns} 个 turn
        </span>
      </div>
      <div className="text-text-primary text-sm leading-[1.7] break-words [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 [&_ul]:my-[0.4rem] [&_ol]:my-[0.4rem] [&_ul]:pl-[1.25rem] [&_ol]:pl-[1.25rem] [&_code]:py-[0.1rem] [&_code]:px-[0.35rem] [&_code]:rounded-md [&_code]:bg-[rgba(34,58,46,0.08)] [&_code]:text-[0.92em]">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.summary}</ReactMarkdown>
      </div>
    </div>
  );
}

export default memo(CompactMessage);
