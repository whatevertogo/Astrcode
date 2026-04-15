import { memo, useRef } from 'react';

import type { ToolCallMessage, ToolStreamMessage } from '../../types';
import {
  extractToolChildSessionTarget,
  extractStructuredArgs,
  extractStructuredJsonOutput,
  extractToolMetadataSummary,
  extractToolShellDisplay,
  formatToolCallSummary,
} from '../../lib/toolDisplay';
import { chevronIcon, infoButton, pillDanger, pillNeutral, pillSuccess } from '../../lib/styles';
import { cn } from '../../lib/utils';
import { useChatScreenContext } from './ChatScreenContext';
import ToolCodePanel from './ToolCodePanel';
import ToolJsonView from './ToolJsonView';
import { useNestedScrollContainment } from './useNestedScrollContainment';

interface ToolCallBlockProps {
  message: ToolCallMessage;
  streams?: ToolStreamMessage[];
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
      return '成功';
    case 'fail':
      return '失败';
    default:
      return '运行中';
  }
}

function streamBadge(stream: ToolStreamMessage['stream']): string {
  return stream === 'stderr' ? pillDanger : pillNeutral;
}

function streamTitle(
  toolName: string,
  stream: ToolStreamMessage['stream'],
  hasShellCommand: boolean
): string {
  if (hasShellCommand && stream === 'stdout') {
    return 'Shell';
  }
  if (stream === 'stderr') {
    return 'stderr';
  }
  return toolName;
}

function resultTextSurface(text: string, tone: 'normal' | 'error') {
  const structuredResult = extractStructuredJsonOutput(text);
  if (structuredResult) {
    return (
      <ToolJsonView
        value={structuredResult.value}
        summary={structuredResult.summary}
        defaultOpen={true}
        scrollMode="inherit"
      />
    );
  }

  return (
    <ToolCodePanel
      title={tone === 'error' ? 'Error output' : 'Result'}
      tone={tone}
      content={text}
      scrollMode="inherit"
    />
  );
}

function ToolCallBlock({ message, streams = [] }: ToolCallBlockProps) {
  const { onOpenChildSession, onOpenSubRun } = useChatScreenContext();
  const viewportRef = useRef<HTMLDivElement>(null);
  useNestedScrollContainment(viewportRef);
  const shellDisplay = extractToolShellDisplay(message.metadata);
  const childSessionTarget = extractToolChildSessionTarget(message.metadata);
  const summary = formatToolCallSummary(
    message.toolName,
    message.args,
    message.status,
    message.metadata
  );
  const structuredArgs = extractStructuredArgs(message.args);
  const metadataSummary = extractToolMetadataSummary(message.metadata);
  const fallbackResult =
    message.error?.trim() || message.output?.trim() || metadataSummary?.message?.trim() || '';
  const structuredFallbackResult = extractStructuredJsonOutput(fallbackResult);
  const defaultOpen = message.status === 'fail';

  return (
    <details
      className="group mb-2 ml-[var(--chat-assistant-content-offset)] block min-w-0 max-w-full animate-block-enter motion-reduce:animate-none"
      open={defaultOpen}
    >
      <summary className="flex min-w-0 cursor-pointer items-center gap-2 py-1.5 font-mono text-[13px] leading-relaxed text-text-secondary list-none [&::-webkit-details-marker]:hidden hover:opacity-85">
        <span className={cn('shrink-0', statusPill(message.status))}>{message.toolName}</span>
        <span className="min-w-0 flex-1 truncate text-text-primary">{summary}</span>
        {childSessionTarget && (
          <button
            type="button"
            className={cn(infoButton, 'min-h-[26px] px-2.5 py-0 text-[11px]')}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (childSessionTarget.openSessionId) {
                void onOpenChildSession(childSessionTarget.openSessionId);
                return;
              }
              if (childSessionTarget.subRunId) {
                void onOpenSubRun(childSessionTarget.subRunId);
              }
            }}
          >
            打开子会话
          </button>
        )}
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
      <div className="mt-2 flex min-w-0 flex-col gap-3 rounded-[18px] border border-border bg-surface-soft px-4 py-3.5 shadow-[0_16px_34px_rgba(15,23,42,0.06)]">
        <div
          ref={viewportRef}
          className="min-w-0 overflow-y-auto overscroll-contain pr-1 max-h-[min(58vh,560px)]"
        >
          <div className="flex min-w-0 flex-col gap-3">
            {streams.length > 0 ? (
              streams.map((streamMessage) => (
                <section key={streamMessage.id} className="flex min-w-0 flex-col gap-2">
                  <div className="flex items-center justify-between gap-3 text-xs text-text-secondary">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className={streamBadge(streamMessage.stream)}>
                        {streamTitle(
                          message.toolName,
                          streamMessage.stream,
                          Boolean(shellDisplay?.command)
                        )}
                      </span>
                      <span className="truncate font-mono text-text-muted">
                        {streamMessage.stream === 'stderr' ? '错误输出' : '工具结果'}
                      </span>
                    </div>
                    <span className="shrink-0 text-text-muted">
                      {statusLabel(streamMessage.status)}
                    </span>
                  </div>
                  {resultTextSurface(
                    streamMessage.content,
                    streamMessage.stream === 'stderr' ? 'error' : 'normal'
                  )}
                </section>
              ))
            ) : fallbackResult ? (
              structuredFallbackResult ? (
                <section className="flex min-w-0 flex-col gap-2">
                  <div className="flex items-center justify-between gap-3 text-xs text-text-secondary">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className={cn('shrink-0', statusPill(message.status))}>结果</span>
                      <span className="truncate font-mono text-text-muted">
                        {structuredFallbackResult.summary}
                      </span>
                    </div>
                    <span className="shrink-0 text-text-muted">{statusLabel(message.status)}</span>
                  </div>
                  <ToolJsonView
                    value={structuredFallbackResult.value}
                    summary={structuredFallbackResult.summary}
                    defaultOpen={true}
                    scrollMode="inherit"
                  />
                </section>
              ) : (
                <section className="flex min-w-0 flex-col gap-2">
                  <div className="flex items-center justify-between gap-3 text-xs text-text-secondary">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className={cn('shrink-0', statusPill(message.status))}>
                        {message.error ? '错误' : '结果'}
                      </span>
                      <span className="truncate font-mono text-text-muted">
                        {shellDisplay?.command ? `$ ${shellDisplay.command}` : message.toolName}
                      </span>
                    </div>
                    <span className="shrink-0 text-text-muted">{statusLabel(message.status)}</span>
                  </div>
                  {resultTextSurface(fallbackResult, message.error ? 'error' : 'normal')}
                </section>
              )
            ) : (
              <div className="rounded-xl border border-dashed border-border bg-white/55 px-3.5 py-3 text-[13px] leading-relaxed text-text-secondary">
                {message.status === 'running' ? '等待工具输出...' : '该工具没有可展示的文本结果。'}
              </div>
            )}
          </div>
        </div>

        {structuredArgs && (
          <details className="group mt-1">
            <summary className="flex cursor-pointer items-center gap-2 text-xs font-medium text-text-secondary list-none [&::-webkit-details-marker]:hidden">
              <span className="text-text-primary">调用参数</span>
              <span className="text-text-muted">{structuredArgs.summary}</span>
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
            <div className="mt-2">
              <ToolJsonView value={structuredArgs.value} summary={structuredArgs.summary} />
            </div>
          </details>
        )}

        {(metadataSummary?.pills?.length ||
          message.durationMs !== undefined ||
          message.truncated) && (
          <div className="flex flex-wrap items-center gap-2 pt-1 text-xs text-text-secondary">
            {metadataSummary?.pills.map((pill) => (
              <span key={pill} className={pillNeutral}>
                {pill}
              </span>
            ))}
            {message.durationMs !== undefined && (
              <span className={pillNeutral}>{message.durationMs} ms</span>
            )}
            {message.truncated && <span className={pillNeutral}>truncated</span>}
          </div>
        )}
      </div>
    </details>
  );
}

export default memo(ToolCallBlock);
