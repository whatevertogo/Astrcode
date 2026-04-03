import React, { Component, memo, useState, useCallback } from 'react';
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

interface CodeBlockProps extends React.ComponentPropsWithoutRef<'code'> {
  inline?: boolean;
}

function CodeBlockComponent({ inline, className, children, ...props }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);
  const match = /language-(\w+)/.exec(className || '');
  const language = match ? match[1] : '';

  const handleCopy = useCallback(() => {
    const code = String(children).replace(/\n$/, '');
    void navigator.clipboard
      .writeText(code)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => {});
  }, [children]);

  if (!inline && match) {
    return (
      <div className={styles.codeBlockWrapper}>
        <div className={styles.codeHeader}>
          <span className={styles.codeLanguage}>{language}</span>
          <button className={styles.copyBtn} onClick={handleCopy} title="Copy Code">
            {copied ? (
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
                <polyline points="20 6 9 17 4 12"></polyline>
              </svg>
            ) : (
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
                <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
              </svg>
            )}
          </button>
        </div>
        <pre className={styles.codeBlock}>
          <code className={className} {...props}>
            {children}
          </code>
        </pre>
      </div>
    );
  }

  return (
    <code className={styles.inlineCode} {...props}>
      {children}
    </code>
  );
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
              <div className={styles.streamingText}>{visibleText}</div>
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
                        return (
                          <CodeBlockComponent className={className} {...props}>
                            {children}
                          </CodeBlockComponent>
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
