import React, { Component, useCallback, useEffect, useRef } from 'react';
import type { Message } from '../../types';
import type { SubRunViewData, ThreadItem } from '../../lib/subRunView';
import UserMessage from './UserMessage';
import AssistantMessage from './AssistantMessage';
import ToolCallBlock from './ToolCallBlock';
import PromptMetricsMessage from './PromptMetricsMessage';
import CompactMessage from './CompactMessage';
import SubRunBlock from './SubRunBlock';
import styles from './MessageList.module.css';

interface MessageListProps {
  sessionId: string | null;
  threadItems: ThreadItem[];
  childSubRuns: SubRunViewData[];
  subRunViews: Map<string, SubRunViewData>;
  contentFingerprint: string;
  emptyStateText?: string;
  onCancelSubRun: (sessionId: string, subRunId: string) => void | Promise<void>;
  onFocusSubRun: (subRunId: string) => void;
  onOpenChildSession: (childSessionId: string) => void | Promise<void>;
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
    console.error('message render failed', {
      kind: this.props.message.kind,
      message: this.props.message,
      error,
    });
  }

  override render() {
    if (this.state.hasError) {
      const { message } = this.props;
      return (
        <div className={styles.renderError}>
          <div className={styles.renderErrorTitle}>消息渲染失败</div>
          <div className={styles.renderErrorMeta}>kind: {message.kind}</div>
          {message.kind === 'toolCall' ? (
            <pre className={styles.renderErrorBody}>
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
            <pre className={styles.renderErrorBody}>{message.summary}</pre>
          ) : message.kind === 'promptMetrics' ? (
            <pre className={styles.renderErrorBody}>
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
            <pre className={styles.renderErrorBody}>
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
            <pre className={styles.renderErrorBody}>
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
          ) : (
            <pre className={styles.renderErrorBody}>{message.text}</pre>
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
  sessionId,
  threadItems,
  childSubRuns,
  subRunViews,
  contentFingerprint,
  emptyStateText,
  onCancelSubRun,
  onFocusSubRun,
  onOpenChildSession,
}: MessageListProps) {
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

  const renderMessageContent = useCallback((msg: Message, hideAvatar: boolean) => {
    if (msg.kind === 'user') {
      return <UserMessage message={msg} />;
    }
    if (msg.kind === 'assistant') {
      return <AssistantMessage message={msg} hideAvatar={hideAvatar} />;
    }
    if (msg.kind === 'toolCall') {
      return <ToolCallBlock message={msg} />;
    }
    if (msg.kind === 'promptMetrics') {
      return <PromptMetricsMessage message={msg} />;
    }
    if (msg.kind === 'compact') {
      return <CompactMessage message={msg} />;
    }
    if (msg.kind === 'subRunStart' || msg.kind === 'subRunFinish') {
      return null;
    }
    return null;
  }, []);

  const renderMessageRow = useCallback(
    (
      msg: Message,
      previousMessage: Message | null,
      options?: {
        key?: string;
        nested?: boolean;
      }
    ) => {
      const isContinuation =
        previousMessage !== null && isAssistantLike(msg) && isAssistantLike(previousMessage);
      const rowClass = [
        isRowNested(options) ? styles.groupMessageRow : styles.messageRow,
        isContinuation ? styles.messageRowContinuation : '',
      ]
        .filter(Boolean)
        .join(' ');

      return (
        <div key={options?.key ?? msg.id} className={rowClass}>
          <MessageBoundary message={msg}>
            {renderMessageContent(msg, isContinuation)}
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
          const previousMessage = previousItem?.kind === 'message' ? previousItem.message : null;
          return renderMessageRow(item.message, previousMessage, {
            key: item.message.id,
            nested: options?.nested,
          });
        }

        const subRunView = subRunViews.get(item.subRunId);
        if (!subRunView) {
          return (
            <div
              key={`subrun-missing-${item.subRunId}`}
              className={isRowNested(options) ? styles.groupMessageRow : styles.messageRow}
            >
              <div className={styles.renderError}>
                <div className={styles.renderErrorTitle}>子执行渲染失败</div>
                <div className={styles.renderErrorMeta}>subRunId: {item.subRunId}</div>
              </div>
            </div>
          );
        }

        const boundaryMessage =
          subRunView.startMessage ?? subRunView.finishMessage ?? subRunView.bodyMessages[0];
        const rowClass = isRowNested(options) ? styles.groupMessageRow : styles.messageRow;
        const subRunBlock = (
          <SubRunBlock
            subRunId={subRunView.subRunId}
            sessionId={sessionId}
            title={subRunView.title}
            startMessage={subRunView.startMessage}
            finishMessage={subRunView.finishMessage}
            threadItems={subRunView.threadItems}
            streamFingerprint={subRunView.streamFingerprint}
            hasDescriptorLineage={subRunView.hasDescriptorLineage}
            renderThreadItems={renderThreadItems}
            onCancelSubRun={onCancelSubRun}
            onFocusSubRun={onFocusSubRun}
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
    [onCancelSubRun, onFocusSubRun, onOpenChildSession, renderMessageRow, sessionId, subRunViews]
  );

  const renderedRows = renderThreadItems(threadItems);
  const childSubRunRows = childSubRuns.map((subRunView) => {
    const boundaryMessage =
      subRunView.startMessage ?? subRunView.finishMessage ?? subRunView.bodyMessages[0];
    const subRunBlock = (
      <SubRunBlock
        subRunId={subRunView.subRunId}
        sessionId={sessionId}
        title={subRunView.title}
        startMessage={subRunView.startMessage}
        finishMessage={subRunView.finishMessage}
        threadItems={subRunView.threadItems}
        streamFingerprint={subRunView.streamFingerprint}
        hasDescriptorLineage={subRunView.hasDescriptorLineage}
        renderThreadItems={renderThreadItems}
        onCancelSubRun={onCancelSubRun}
        onFocusSubRun={onFocusSubRun}
        onOpenChildSession={onOpenChildSession}
        displayMode="directory"
      />
    );

    return (
      <div key={`child-subrun-${subRunView.subRunId}`} className={styles.messageRow}>
        {boundaryMessage ? (
          <MessageBoundary message={boundaryMessage}>{subRunBlock}</MessageBoundary>
        ) : (
          subRunBlock
        )}
      </div>
    );
  });

  return (
    <div ref={listRef} className={styles.list} onScroll={updateStickiness}>
      {threadItems.length === 0 && childSubRuns.length === 0 && (
        <div className={styles.empty}>{emptyStateText ?? '向 AstrCode 提问，开始对话...'}</div>
      )}
      {renderedRows}
      {childSubRuns.length > 0 && (
        <section className={styles.childSubRunSection}>
          <div className={styles.childSubRunHeader}>下一级子执行</div>
          <div className={styles.childSubRunList}>{childSubRunRows}</div>
        </section>
      )}
      <div ref={bottomRef} />
    </div>
  );
}
