import React, { Component, useCallback, useEffect, useRef } from 'react';
import type { Message } from '../../types';
import UserMessage from './UserMessage';
import AssistantMessage from './AssistantMessage';
import ToolCallBlock from './ToolCallBlock';
import CompactMessage from './CompactMessage';
import styles from './MessageList.module.css';

interface MessageListProps {
  messages: Message[];
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

function isSubRunLifecycleMessage(message: Message): boolean {
  return message.kind === 'subRunStart' || message.kind === 'subRunFinish';
}

function isNestedAgentMessage(message: Message): boolean {
  return Boolean(message.subRunId);
}

export default function MessageList({ messages }: MessageListProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const shouldStickToBottomRef = useRef(true);
  const previousMessageCountRef = useRef(0);

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
      previousMessageCountRef.current === 0 || shouldStickToBottomRef.current;
    previousMessageCountRef.current = messages.length;
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
  }, [messages, stickToBottom, updateStickiness]);

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
        options?.nested ? styles.groupMessageRow : styles.messageRow,
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

  const renderedRows: React.ReactNode[] = [];
  for (let index = 0; index < messages.length; ) {
    const message = messages[index];
    if (!isNestedAgentMessage(message)) {
      renderedRows.push(renderMessageRow(message, index > 0 ? messages[index - 1] : null));
      index += 1;
      continue;
    }

    const group = [message];
    let nextIndex = index + 1;
    while (nextIndex < messages.length) {
      const nextMessage = messages[nextIndex];
      if (nextMessage.subRunId !== message.subRunId) {
        break;
      }
      group.push(nextMessage);
      nextIndex += 1;
    }

    const startMessage = group.find(
      (item): item is Extract<Message, { kind: 'subRunStart' }> => item.kind === 'subRunStart'
    );
    const finishMessage = group.find(
      (item): item is Extract<Message, { kind: 'subRunFinish' }> => item.kind === 'subRunFinish'
    );
    const bodyMessages = group.filter((item) => !isSubRunLifecycleMessage(item));
    const status =
      finishMessage === undefined
        ? 'running'
        : typeof finishMessage.result.status === 'string'
          ? finishMessage.result.status
          : 'failed';
    const metrics =
      finishMessage !== undefined
        ? `${finishMessage.stepCount} steps · ${finishMessage.estimatedTokens} tokens`
        : startMessage?.storageMode === 'independentSession'
          ? 'independent session'
          : 'shared session';
    const title =
      startMessage?.agentProfile ??
      finishMessage?.agentProfile ??
      message.agentProfile ??
      message.agentId ??
      '子会话';

    renderedRows.push(
      <div
        key={`agent-group-${message.subRunId ?? 'unknown'}-${index}`}
        className={styles.agentGroup}
      >
        <div className={styles.agentGroupHeader}>
          <span className={styles.agentGroupLabel}>子会话</span>
          <span className={styles.agentGroupTitle}>{title}</span>
          <span className={styles.agentGroupLabel}>{status}</span>
          <span className={styles.agentGroupTitle}>{metrics}</span>
        </div>
        <div className={styles.agentGroupBody}>
          {bodyMessages.map((groupMessage, groupIndex) =>
            renderMessageRow(groupMessage, groupIndex > 0 ? bodyMessages[groupIndex - 1] : null, {
              key: groupMessage.id,
              nested: true,
            })
          )}
        </div>
      </div>
    );
    index = nextIndex;
  }

  return (
    <div ref={listRef} className={styles.list} onScroll={updateStickiness}>
      {messages.length === 0 && <div className={styles.empty}>向 AstrCode 提问，开始对话...</div>}
      {renderedRows}
      <div ref={bottomRef} />
    </div>
  );
}
