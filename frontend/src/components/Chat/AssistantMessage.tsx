import React, { Component, memo, useState, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { AssistantMessage as AssistantMessageType } from '../../types';
import styles from './AssistantMessage.module.css';

interface AssistantMessageProps {
  message: AssistantMessageType;
  hideAvatar?: boolean;
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

function CopyButton({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    void navigator.clipboard
      .writeText(code)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => {});
  }, [code]);

  return (
    <button className={styles.copyBtn} onClick={handleCopy} title="Copy Code">
      {copied ? (
        <>
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
          <span>Copied!</span>
        </>
      ) : (
        <>
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
          <span>Copy</span>
        </>
      )}
    </button>
  );
}

interface CodeBlockRendererProps extends React.ComponentPropsWithoutRef<'code'> {
  node?: { parent?: { tagName?: string } };
  inline?: boolean;
}

function CodeBlockRenderer({ node, className, children, ...props }: CodeBlockRendererProps) {
  const match = /language-(\w+)/.exec(className || '');
  const language = match ? match[1] : '';

  const isInline = !match && !String(children).includes('\n') && node?.parent?.tagName !== 'pre';

  if (isInline) {
    return (
      <code className={styles.inlineCode} {...props}>
        {children}
      </code>
    );
  }

  const codeText = String(children).replace(/\n$/, '');

  return (
    <div className={styles.codeBlockWrapper}>
      <div className={styles.codeHeader}>
        <span className={styles.codeLanguage}>{language || 'text'}</span>
        <CopyButton code={codeText} />
      </div>
      <pre className={styles.codeBlock} {...props}>
        <code className={className}>{children}</code>
      </pre>
    </div>
  );
}

const markdownComponents: Partial<import('react-markdown').Components> = {
  pre: ({ children }: React.PropsWithChildren) => <>{children}</>,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/no-unsafe-assignment
  code: CodeBlockRenderer as any,
};

function AssistantMessage({ message, hideAvatar }: AssistantMessageProps) {
  const { visibleText, thinkingBlocks } = React.useMemo(
    () => extractThinkingBlocks(message.text, message.reasoningText),
    [message.text, message.reasoningText]
  );

  return (
    <div className={styles.wrapper}>
      <div
        className={styles.avatar}
        aria-hidden="true"
        style={{ opacity: hideAvatar ? 0 : 1, visibility: hideAvatar ? 'hidden' : 'visible' }}
      >
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
          {thinkingBlocks.map((block, index) => (
            <details
              key={`${message.id}-thinking-${index}`}
              className={styles.thinkingBlock}
              open={message.streaming}
            >
              <summary className={styles.thinkingSummary}>
                <span className={styles.thinkingIcon}>
                  <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z" />
                    <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z" />
                    <path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4" />
                    <path d="M17.599 6.5a3 3 0 0 0 .399-1.375" />
                    <path d="M6.003 5.125A3 3 0 0 0 6.401 6.5" />
                    <path d="M3.477 10.896a4 4 0 0 1 .585-.396" />
                    <path d="M19.938 10.5a4 4 0 0 1 .585.396" />
                    <path d="M6 18a4 4 0 0 1-1.967-.516" />
                    <path d="M19.967 17.484A4 4 0 0 1 18 18" />
                  </svg>
                </span>
                <span>Thinking</span>
                <span className={styles.thinkingChevron}>
                  <svg
                    width="16"
                    height="16"
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
              <div className={styles.thinkingContent}>
                <MarkdownGuard fallback={block}>
                  <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
                    {block}
                  </ReactMarkdown>
                </MarkdownGuard>
              </div>
            </details>
          ))}
          {visibleText ? (
            <MarkdownGuard fallback={visibleText}>
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
                {visibleText}
              </ReactMarkdown>
            </MarkdownGuard>
          ) : null}
          {message.streaming && <span className={styles.cursor}>▋</span>}
        </div>
      </div>
    </div>
  );
}

export default memo(AssistantMessage);
