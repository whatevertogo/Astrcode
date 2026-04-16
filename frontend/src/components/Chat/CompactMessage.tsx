import { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { CompactMessage as CompactMessageType } from '../../types';
import { pillSuccess } from '../../lib/styles';

interface CompactMessageProps {
  message: CompactMessageType;
}

function CompactMessage({ message }: CompactMessageProps) {
  const triggerLabel =
    message.trigger === 'manual'
      ? '手动压缩'
      : message.trigger === 'deferred'
        ? '延后压缩'
        : '自动压缩';
  const modeLabel =
    message.meta.mode === 'incremental'
      ? '增量'
      : message.meta.mode === 'retry_salvage'
        ? '抢救回退'
        : '全量';

  return (
    <div className="ml-[var(--chat-assistant-content-offset)] min-w-0 max-w-full rounded-[18px] border border-success/25 bg-success-soft px-4 pb-4 pt-3.5 shadow-soft">
      <div className="mb-3 flex flex-wrap items-center gap-2.5">
        <span className={pillSuccess}>{triggerLabel}</span>
        <span className="rounded-full border border-success/20 bg-white/65 px-2.5 py-1 text-[11px] font-medium text-text-secondary">
          {modeLabel}
        </span>
        <span className="text-text-muted text-xs">
          保留最近 {message.preservedRecentTurns} 个 turn
        </span>
        {message.meta.instructionsPresent ? (
          <span className="text-text-muted text-xs">含自定义指令</span>
        ) : null}
        {message.meta.fallbackUsed ? (
          <span className="text-text-muted text-xs">
            fallback
            {message.meta.retryCount > 0 ? ` · 重试 ${message.meta.retryCount} 次` : ''}
          </span>
        ) : message.meta.retryCount > 0 ? (
          <span className="text-text-muted text-xs">重试 {message.meta.retryCount} 次</span>
        ) : null}
      </div>
      <div className="min-w-0 max-w-full break-words text-sm leading-[1.7] text-text-primary prose-chat [&_ol]:my-[0.4rem] [&_ol]:pl-[1.25rem] [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 [&_ul]:my-[0.4rem] [&_ul]:pl-[1.25rem]">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.summary}</ReactMarkdown>
      </div>
      <div className="mt-3 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-text-muted">
        <span>input units: {message.meta.inputUnits}</span>
        <span>summary chars: {message.meta.outputSummaryChars}</span>
      </div>
    </div>
  );
}

export default memo(CompactMessage);
