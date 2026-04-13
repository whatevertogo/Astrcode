import React, { Component, useCallback, useEffect, useRef } from 'react';
import type { Message, SubRunViewData, ThreadItem } from '../../types';
import { emptyStateSurface, errorSurface } from '../../lib/styles';
import { cn } from '../../lib/utils';
import AssistantMessage from './AssistantMessage';
import CompactMessage from './CompactMessage';
import SubRunBlock from './SubRunBlock';
import ToolCallBlock from './ToolCallBlock';
import UserMessage from './UserMessage';
import { useChatScreenContext } from './ChatScreenContext';
import { logger } from '../../lib/logger';

interface MessageListProps {
  threadItems: ThreadItem[];
  childSubRuns: SubRunViewData[];
  subRunViews: Map<string, SubRunViewData>;
  contentFingerprint: string;
}

interface MessageBoundaryProps {
  message: Message;
  children: React.ReactNode;
}

interface MessageBoundaryState {
  hasError: boolean;
}

class MessageBoundary extends Component<MessageBoundaryProps, MessageBoundaryState> {
  state: MessageBoundaryState = { hasError: false };

  static getDerivedStateFromError(): MessageBoundaryState {
    return { hasError: true };
  }

  override componentDidCatch(error: Error) {
    logger.error('MessageList', 'message render failed', {
      kind: this.props.message.kind,
      message: this.props.message,
      error,
    });
  }

  override render() {
    if (this.state.hasError) {
      const { message } = this.props;
      return (
        <div className={errorSurface}>
          <div className="mb-1.5 text-[13px] font-semibold">消息渲染失败</div>
          <div className="mb-2 text-xs text-danger/70">kind: {message.kind}</div>
          {message.kind === 'toolCall' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {JSON.stringify(
                {
                  toolCallId: message.toolCallId,
                  toolName: message.toolName,
                  status: message.status,
                  durationMs: message.durationMs,
                  error: message.error,
                },
                null,
                2
              )}
            </pre>
          ) : message.kind === 'compact' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {message.summary}
            </pre>
          ) : message.kind === 'promptMetrics' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {JSON.stringify(
                {
                  stepIndex: message.stepIndex,
                  estimatedTokens: message.estimatedTokens,
                  providerInputTokens: message.providerInputTokens,
                  providerOutputTokens: message.providerOutputTokens,
                  cacheReadInputTokens: message.cacheReadInputTokens,
                  cacheCreationInputTokens: message.cacheCreationInputTokens,
                },
                null,
                2
              )}
            </pre>
          ) : message.kind === 'subRunStart' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {JSON.stringify(
                {
                  subRunId: message.subRunId,
                  storageMode: message.storageMode,
                  agentProfile: message.agentProfile,
                },
                null,
                2
              )}
            </pre>
          ) : message.kind === 'subRunFinish' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {JSON.stringify(
                {
                  subRunId: message.subRunId,
                  status: message.result.status,
                  stepCount: message.stepCount,
                  estimatedTokens: message.estimatedTokens,
                },
                null,
                2
              )}
            </pre>
          ) : message.kind === 'childSessionNotification' ? (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {JSON.stringify(
                {
                  subRunId: message.childRef.subRunId,
                  kind: message.notificationKind,
                  status: message.status,
                  openSessionId: message.childRef.openSessionId,
                },
                null,
                2
              )}
            </pre>
          ) : (
            <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere text-xs leading-relaxed">
              {message.text}
            </pre>
          )}
        </div>
      );
    }

    return this.props.children;
  }
}

function isAssistantLike(message: Message): boolean {
  return message.kind === 'assistant' || message.kind === 'toolCall';
}

function isRowNested(options?: { nested?: boolean }): boolean {
  return options?.nested === true;
}

export default function MessageList({
  threadItems,
  childSubRuns,
  subRunViews,
  contentFingerprint,
}: MessageListProps) {
  const {
    sessionId,
    activeSubRunPath,
    isChildSession,
    onCancelSubRun,
    onOpenSubRun,
    onOpenChildSession,
  } = useChatScreenContext();
  const listRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const shouldStickToBottomRef = useRef(true);
  const previousContentFingerprintRef = useRef('');

  const updateStickiness = useCallback(() => {
    const container = listRef.current;
    if (!container) {
      shouldStickToBottomRef.current = true;
      return;
    }
    const distanceFromBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    shouldStickToBottomRef.current = distanceFromBottom <= 48;
  }, []);

  const stickToBottom = useCallback(() => {
    const container = listRef.current;
    if (!container) {
      return;
    }
    // 使用 scrollTop 直接贴底，避免 WebView2 对 scrollIntoView + smooth 的不稳定行为。
    container.scrollTop = container.scrollHeight;
  }, []);

  useEffect(() => {
    updateStickiness();
  }, [updateStickiness]);

  useEffect(() => {
    const shouldAutoScroll =
      previousContentFingerprintRef.current === '' || shouldStickToBottomRef.current;
    previousContentFingerprintRef.current = contentFingerprint;
    if (!shouldAutoScroll) {
      return;
    }
    const rafId = window.requestAnimationFrame(() => {
      if (bottomRef.current && listRef.current) {
        stickToBottom();
      } else {
        bottomRef.current?.scrollIntoView();
      }
      updateStickiness();
    });
    return () => window.cancelAnimationFrame(rafId);
  }, [contentFingerprint, stickToBottom, updateStickiness]);

  const renderMessageContent = useCallback(
    (
      msg: Message,
      hideAvatar: boolean,
      metrics?: Message,
      options?: {
        nested?: boolean;
      }
    ) => {
      if (msg.kind === 'user') {
        return <UserMessage message={msg} />;
      }
      if (msg.kind === 'assistant') {
        const promptMetrics = metrics?.kind === 'promptMetrics' ? metrics : undefined;
        const presentation =
          !isChildSession && activeSubRunPath.length === 0 && options?.nested !== true
            ? 'root'
            : 'subRun';
        return (
          <AssistantMessage
            message={msg}
            hideAvatar={hideAvatar}
            metrics={promptMetrics}
            presentation={presentation}
          />
        );
      }
      if (msg.kind === 'toolCall') {
        return <ToolCallBlock message={msg} />;
      }
      if (msg.kind === 'promptMetrics') {
        return null;
      }
      if (msg.kind === 'compact') {
        return <CompactMessage message={msg} />;
      }
      if (msg.kind === 'subRunStart' || msg.kind === 'subRunFinish') {
        return null;
      }
      return null;
    },
    [activeSubRunPath.length, isChildSession]
  );

  const renderMessageRow = useCallback(
    (
      msg: Message,
      previousMessage: Message | null,
      nextMessage: Message | null,
      options?: {
        key?: string;
        nested?: boolean;
      },
      metricsOverride?: Message
    ) => {
      const isContinuation =
        previousMessage !== null && isAssistantLike(msg) && isAssistantLike(previousMessage);
      const metricsToAttach =
        metricsOverride ??
        (msg.kind === 'assistant' && nextMessage?.kind === 'promptMetrics'
          ? nextMessage
          : undefined);

      return (
        <div
          key={options?.key ?? msg.id}
          className={cn(
            isRowNested(options) ? 'w-full' : 'mx-auto w-[min(100%,var(--chat-content-max-width))]',
            'min-w-0 transition-[margin-top] duration-200 ease-out',
            isContinuation && '-mt-4'
          )}
        >
          <MessageBoundary message={msg}>
            {renderMessageContent(msg, isContinuation, metricsToAttach, options)}
          </MessageBoundary>
        </div>
      );
    },
    [renderMessageContent]
  );

  const renderThreadItems = useCallback(
    (
      items: ThreadItem[],
      options?: {
        nested?: boolean;
      }
    ): React.ReactNode[] =>
      items.map((item, index) => {
        if (item.kind === 'message') {
          const previousItem = items[index - 1];
          const nextItem = items[index + 1];
          const previousMessage = previousItem?.kind === 'message' ? previousItem.message : null;
          const nextMessage = nextItem?.kind === 'message' ? nextItem.message : null;

          if (item.message.kind === 'promptMetrics') {
            return null;
          }

          let metricsToAttach: Message | undefined;
          if (item.message.kind === 'assistant') {
            let hasMoreAssistantInTurn = false;
            const currentTurnId = item.message.turnId;

            for (let nextIndex = index + 1; nextIndex < items.length; nextIndex += 1) {
              const nextThreadItem = items[nextIndex];
              if (nextThreadItem.kind !== 'message') {
                continue;
              }
              if (
                nextThreadItem.message.kind === 'assistant' &&
                nextThreadItem.message.turnId === currentTurnId
              ) {
                hasMoreAssistantInTurn = true;
                break;
              }
              if (
                nextThreadItem.message.kind === 'user' ||
                (nextThreadItem.message.kind === 'assistant' &&
                  nextThreadItem.message.turnId !== currentTurnId)
              ) {
                break;
              }
            }

            if (!hasMoreAssistantInTurn) {
              for (let nextIndex = index + 1; nextIndex < items.length; nextIndex += 1) {
                const nextThreadItem = items[nextIndex];
                if (nextThreadItem.kind !== 'message') {
                  continue;
                }
                if (nextThreadItem.message.kind === 'promptMetrics') {
                  metricsToAttach = nextThreadItem.message;
                  break;
                }
                if (
                  nextThreadItem.message.kind === 'assistant' ||
                  nextThreadItem.message.kind === 'user'
                ) {
                  break;
                }
              }
            }
          }

          return renderMessageRow(
            item.message,
            previousMessage,
            nextMessage,
            {
              key: item.message.id,
              nested: options?.nested,
            },
            metricsToAttach
          );
        }

        const subRunView = subRunViews.get(item.subRunId);
        if (!subRunView) {
          return (
            <div
              key={`subrun-missing-${item.subRunId}`}
              className={
                isRowNested(options)
                  ? 'min-w-0 w-full'
                  : 'mx-auto min-w-0 w-[min(100%,var(--chat-content-max-width))]'
              }
            >
              <div className={errorSurface}>
                <div className="mb-1.5 text-[13px] font-semibold">子执行渲染失败</div>
                <div className="mb-2 text-xs text-danger/70">subRunId: {item.subRunId}</div>
              </div>
            </div>
          );
        }

        const boundaryMessage =
          subRunView.startMessage ?? subRunView.finishMessage ?? subRunView.bodyMessages[0];
        const rowClass = isRowNested(options)
          ? 'min-w-0 w-full'
          : 'mx-auto min-w-0 w-[min(100%,var(--chat-content-max-width))]';
        const subRunBlock = (
          <SubRunBlock
            subRunId={subRunView.subRunId}
            sessionId={sessionId}
            childSessionId={subRunView.childSessionId}
            title={subRunView.title}
            startMessage={subRunView.startMessage}
            finishMessage={subRunView.finishMessage}
            threadItems={subRunView.threadItems}
            streamFingerprint={subRunView.streamFingerprint}
            hasDescriptorLineage={subRunView.hasDescriptorLineage}
            renderThreadItems={renderThreadItems}
            onCancelSubRun={onCancelSubRun}
            onFocusSubRun={onOpenSubRun}
            onOpenChildSession={onOpenChildSession}
          />
        );

        return (
          <div key={`subrun-${subRunView.subRunId}`} className={rowClass}>
            {boundaryMessage ? (
              <MessageBoundary message={boundaryMessage}>{subRunBlock}</MessageBoundary>
            ) : (
              subRunBlock
            )}
          </div>
        );
      }),
    [onCancelSubRun, onOpenChildSession, onOpenSubRun, renderMessageRow, sessionId, subRunViews]
  );

  const renderedRows = renderThreadItems(threadItems);
  const childSubRunRows = childSubRuns.map((subRunView) => {
    const boundaryMessage =
      subRunView.startMessage ?? subRunView.finishMessage ?? subRunView.bodyMessages[0];
    const subRunBlock = (
      <SubRunBlock
        subRunId={subRunView.subRunId}
        sessionId={sessionId}
        childSessionId={subRunView.childSessionId}
        title={subRunView.title}
        startMessage={subRunView.startMessage}
        finishMessage={subRunView.finishMessage}
        threadItems={subRunView.threadItems}
        streamFingerprint={subRunView.streamFingerprint}
        hasDescriptorLineage={subRunView.hasDescriptorLineage}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={onCancelSubRun}
        onFocusSubRun={onOpenSubRun}
        onOpenChildSession={onOpenChildSession}
        displayMode="directory"
      />
    );

    return (
      <div
        key={`child-subrun-${subRunView.subRunId}`}
        className="mx-auto min-w-0 w-[min(100%,var(--chat-content-max-width))]"
      >
        {boundaryMessage ? (
          <MessageBoundary message={boundaryMessage}>{subRunBlock}</MessageBoundary>
        ) : (
          subRunBlock
        )}
      </div>
    );
  });

  return (
    <div
      ref={listRef}
      className="flex min-w-0 flex-1 flex-col gap-[22px] overflow-x-hidden overflow-y-auto bg-panel-bg px-[var(--chat-content-horizontal-padding)] py-7 max-sm:gap-[18px] max-sm:px-[var(--chat-content-horizontal-padding-mobile)] max-sm:pb-2 max-sm:pt-[18px]"
      onScroll={updateStickiness}
    >
      {threadItems.length === 0 && childSubRuns.length === 0 && (
        <div
          className={cn(
            emptyStateSurface,
            'mx-auto mt-[90px] w-[min(100%,var(--chat-content-max-width))] max-sm:mt-[54px]'
          )}
        >
          {activeSubRunPath.length > 0 ? '等待该子执行输出...' : '向 AstrCode 提问，开始对话...'}
        </div>
      )}
      {renderedRows}
      {childSubRuns.length > 0 && (
        <section className="mx-auto mt-1 flex w-[min(100%,var(--chat-content-max-width))] flex-col gap-3">
          <div className="text-xs leading-snug tracking-[0.02em] text-text-secondary">
            下一级子执行
          </div>
          <div className="flex flex-col gap-[18px]">{childSubRunRows}</div>
        </section>
      )}
      <div ref={bottomRef} />
    </div>
  );
}
