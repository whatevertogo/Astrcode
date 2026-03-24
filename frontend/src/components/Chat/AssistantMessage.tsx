import React, { Component, memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { AssistantMessage as AssistantMessageType } from '../../types';
import styles from './AssistantMessage.module.css';

interface AssistantMessageProps {
  message: AssistantMessageType;
}

interface MarkdownGuardProps {
  fallback: string;
  children: React.ReactNode;
}

interface MarkdownGuardState {
  hasError: boolean;
}

class MarkdownGuard extends Component<MarkdownGuardProps, MarkdownGuardState> {
  state: MarkdownGuardState = { hasError: false };

  static getDerivedStateFromError(): MarkdownGuardState {
    return { hasError: true };
  }

  override render() {
    if (this.state.hasError) {
      return <pre className={styles.fallbackText}>{this.props.fallback}</pre>;
    }

    return this.props.children;
  }
}

function extractThinkingBlocks(
  text: string,
  explicitReasoning?: string
): {
  visibleText: string;
  thinkingBlocks: string[];
} {
  const thinkingBlocks: string[] = [];
  if (explicitReasoning?.trim()) {
    thinkingBlocks.push(explicitReasoning.trim());
  }
  const visibleText = text
    .replace(/<think>([\s\S]*?)<\/think>/gi, (_match, content: string) => {
      const normalized = content.trim();
      if (normalized) {
        if (!thinkingBlocks.includes(normalized)) {
          thinkingBlocks.push(normalized);
        }
      }
      return '';
    })
    .trim();

  return { visibleText, thinkingBlocks };
}

function AssistantMessage({ message }: AssistantMessageProps) {
  const { visibleText, thinkingBlocks } = extractThinkingBlocks(
    message.text,
    message.reasoningText
  );

  return (
    <div className={styles.wrapper}>
      <div className={styles.avatar} aria-hidden="true">
        <svg viewBox="0 0 20 20">
          <rect
            x="3.25"
            y="3.25"
            width="13.5"
            height="13.5"
            rx="3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
          />
          <path
            d="M8 8h4M8 10h4M8 12h2.5"
            fill="none"
            stroke="currentColor"
            strokeLinecap="round"
            strokeWidth="1.4"
          />
        </svg>
      </div>
      <div className={styles.body}>
        <div
          className={`${styles.content} ${message.streaming ? styles.contentStreaming : ''}`}
          data-streaming={message.streaming ? 'true' : 'false'}
        >
          {message.streaming ? (
            <>
              {thinkingBlocks.map((block, index) => (
                <details
                  key={`${message.id}-thinking-${index}`}
                  className={styles.thinkingBlock}
                  open
                >
                  <summary className={styles.thinkingSummary}>Thinking</summary>
                  <pre className={styles.thinkingContent}>{block}</pre>
                </details>
              ))}
              <div className={styles.streamingText}>{message.text}</div>
            </>
          ) : (
            <>
              {thinkingBlocks.map((block, index) => (
                <details key={`${message.id}-thinking-${index}`} className={styles.thinkingBlock}>
                  <summary className={styles.thinkingSummary}>Thinking</summary>
                  <pre className={styles.thinkingContent}>{block}</pre>
                </details>
              ))}
              {visibleText ? (
                <MarkdownGuard fallback={visibleText}>
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm]}
                    components={{
                      code({ className, children, ...props }) {
                        const isBlock = className?.startsWith('language-');
                        if (isBlock) {
                          return (
                            <pre className={styles.codeBlock}>
                              <code className={className} {...props}>
                                {children}
                              </code>
                            </pre>
                          );
                        }
                        return (
                          <code className={styles.inlineCode} {...props}>
                            {children}
                          </code>
                        );
                      },
                    }}
                  >
                    {visibleText}
                  </ReactMarkdown>
                </MarkdownGuard>
              ) : null}
            </>
          )}
          {message.streaming && <span className={styles.cursor}>▋</span>}
        </div>
      </div>
    </div>
  );
}

export default memo(AssistantMessage);
