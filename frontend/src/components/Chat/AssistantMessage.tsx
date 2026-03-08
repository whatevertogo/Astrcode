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

function extractThinkingBlocks(text: string): {
  visibleText: string;
  thinkingBlocks: string[];
} {
  const thinkingBlocks: string[] = [];
  const visibleText = text
    .replace(/<think>([\s\S]*?)<\/think>/gi, (_match, content: string) => {
      const normalized = content.trim();
      if (normalized) {
        thinkingBlocks.push(normalized);
      }
      return '';
    })
    .trim();

  return { visibleText, thinkingBlocks };
}

function AssistantMessage({ message }: AssistantMessageProps) {
  const { visibleText, thinkingBlocks } = extractThinkingBlocks(message.text);

  return (
    <div className={styles.wrapper}>
      <div className={styles.bubble}>
        <div className={styles.label}>Assistant</div>
        <div className={styles.content}>
          {message.streaming ? (
            <div className={styles.streamingText}>{message.text}</div>
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
