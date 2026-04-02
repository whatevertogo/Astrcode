import React, { Component, useCallback, useEffect, useRef } from 'react';
import type { Message } from '../../types';
import UserMessage from './UserMessage';
import AssistantMessage from './AssistantMessage';
import ToolCallBlock from './ToolCallBlock';
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
          ) : (
            <pre className={styles.renderErrorBody}>{message.text}</pre>
          )}
        </div>
      );
    }

    return this.props.children;
  }
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

  useEffect(() => {
    updateStickiness();
  }, [updateStickiness]);

  useEffect(() => {
    const shouldAutoScroll =
      previousMessageCountRef.current === 0 || shouldStickToBottomRef.current;
    previousMessageCountRef.current = messages.length;
    if (shouldAutoScroll) {
      bottomRef.current?.scrollIntoView();
      updateStickiness();
    }
  }, [messages, updateStickiness]);

  return (
    <div ref={listRef} className={styles.list} onScroll={updateStickiness}>
      {messages.length === 0 && <div className={styles.empty}>向 AstrCode 提问，开始对话...</div>}
      {messages.map((msg) => {
        if (msg.kind === 'user') {
          return (
            <div key={msg.id} className={styles.messageRow}>
              <MessageBoundary message={msg}>
                <UserMessage message={msg} />
              </MessageBoundary>
            </div>
          );
        }
        if (msg.kind === 'assistant') {
          return (
            <div key={msg.id} className={styles.messageRow}>
              <MessageBoundary message={msg}>
                <AssistantMessage message={msg} />
              </MessageBoundary>
            </div>
          );
        }
        if (msg.kind === 'toolCall') {
          return (
            <div key={msg.id} className={styles.messageRow}>
              <MessageBoundary message={msg}>
                <ToolCallBlock message={msg} />
              </MessageBoundary>
            </div>
          );
        }
        return null;
      })}
      <div ref={bottomRef} />
    </div>
  );
}
