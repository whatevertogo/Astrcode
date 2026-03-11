import React, { Component, memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { AssistantMessage as AssistantMessageType } from '../../types';
import ThinkingBlock from './ThinkingBlock';
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

function AssistantMessage({ message }: AssistantMessageProps) {
  return (
    <div className={styles.wrapper}>
      <div className={styles.bubble}>
        <div className={styles.label}>Assistant</div>
        <div className={styles.content}>
          {message.reasoningText ? (
            <ThinkingBlock
              reasoningText={message.reasoningText}
              streaming={Boolean(message.reasoningStreaming)}
            />
          ) : null}
          {message.streaming ? (
            <div className={styles.streamingText}>{message.text}</div>
          ) : (
            <>
              {message.text ? (
                <MarkdownGuard fallback={message.text}>
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
                    {message.text}
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
