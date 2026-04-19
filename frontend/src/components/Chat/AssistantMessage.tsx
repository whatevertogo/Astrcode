import React, { Component, memo, useState, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { AssistantMessage as AssistantMessageType, PromptMetricsMessage } from '../../types';
import {
  assistantAvatar,
  chevronIcon,
  codeBlockContent,
  codeBlockHeader,
  codeBlockShell,
  expandableBody,
  ghostIconButton,
} from '../../lib/styles';
import { calculateCacheHitRatePercent, calculatePromptReuseRatePercent, cn } from '../../lib/utils';

interface AssistantMessageProps {
  message: AssistantMessageType;
  hideAvatar?: boolean;
  metrics?: PromptMetricsMessage;
  presentation?: 'root' | 'subRun';
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
      return (
        <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere font-inherit text-inherit">
          {this.props.fallback}
        </pre>
      );
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
    .replace(/<think([\s\S]*?)<\/think>/gi, (_match, content: string) => {
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
    <button
      className={cn(
        ghostIconButton,
        'h-7 gap-1.5 rounded px-2 text-[13px] opacity-0 translate-y-0.5 group-hover:translate-y-0 group-hover:opacity-100'
      )}
      onClick={handleCopy}
      title="复制代码"
    >
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
          <span>已复制</span>
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
          <span>复制</span>
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
      <code className={className} {...props}>
        {children}
      </code>
    );
  }

  const codeText = String(children).replace(/\n$/, '');

  return (
    <div className={codeBlockShell}>
      <div className={codeBlockHeader}>
        <span className="text-xs lowercase">{language || 'text'}</span>
        <CopyButton code={codeText} />
      </div>
      <pre className={codeBlockContent} {...props}>
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

interface MarkdownContentProps {
  text: string;
  defer?: boolean;
}

const MarkdownContent = memo(function MarkdownContent({
  text,
  defer = false,
}: MarkdownContentProps) {
  // 流式阶段保留 Markdown 渲染，但把重解析下放到低优先级，避免每个 delta 都阻塞输入和滚动。
  const deferredText = React.useDeferredValue(text);
  const renderedText = defer ? deferredText : text;

  return (
    <MarkdownGuard fallback={renderedText}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {renderedText}
      </ReactMarkdown>
    </MarkdownGuard>
  );
});

function formatTokenCount(value?: number): string {
  if (value === undefined) {
    return '—';
  }
  if (value >= 1000) {
    return `${Math.round(value / 1000)}k`;
  }
  return value.toLocaleString();
}

function getCacheIndicator(metrics?: PromptMetricsMessage): React.ReactNode {
  const providerHitRate = calculateCacheHitRatePercent(metrics);
  if (providerHitRate !== null) {
    if (providerHitRate >= 80) {
      return (
        <span className="ml-2 font-medium text-cache-high">🟢 KV 缓存 {providerHitRate}%</span>
      );
    }
    if (providerHitRate >= 30) {
      return (
        <span className="ml-2 font-medium text-cache-medium">🟡 KV 缓存 {providerHitRate}%</span>
      );
    }
    if (providerHitRate > 0) {
      return <span className="ml-2 font-medium text-cache-low">🟠 KV 缓存 {providerHitRate}%</span>;
    }
    return <span className="ml-2 font-semibold text-cache-none">🔴 KV 缓存 0%</span>;
  }

  const promptReuseRate = calculatePromptReuseRatePercent(metrics);
  if (promptReuseRate === null) {
    return null;
  }
  return (
    <span className="ml-2 font-medium text-cache-medium">🧩 Prompt 复用 {promptReuseRate}%</span>
  );
}

function AssistantMessage({
  message,
  hideAvatar,
  metrics,
  presentation = 'root',
}: AssistantMessageProps) {
  const { visibleText, thinkingBlocks } = React.useMemo(
    () => extractThinkingBlocks(message.text, message.reasoningText),
    [message.text, message.reasoningText]
  );
  const streaming = message.streaming;
  const useThinkingChrome = presentation === 'root';
  const inlineReasoningText =
    !useThinkingChrome && thinkingBlocks.length > 0 ? thinkingBlocks.join('\n\n') : '';

  return (
    <div className="flex items-start gap-4 animate-message-enter max-sm:gap-3 motion-reduce:animate-none">
      <div
        className={assistantAvatar}
        aria-hidden="true"
        style={{ opacity: hideAvatar ? 0 : 1, visibility: hideAvatar ? 'hidden' : 'visible' }}
      >
        <svg viewBox="0 0 20 20" className="w-4 h-4">
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
      <div className="min-w-0 flex-1 pt-0.5">
        <div
          className={cn(
            'relative min-w-0 max-w-full overflow-wrap-anywhere bg-transparent py-2 text-text-primary prose-chat'
          )}
          data-streaming={message.streaming ? 'true' : 'false'}
        >
          {useThinkingChrome
            ? thinkingBlocks.map((block, index) => (
                <details
                  key={`${message.id}-thinking-${index}`}
                  className="mb-3.5 bg-transparent border-none rounded-0 overflow-visible group"
                  open={message.streaming}
                >
                  <summary className="inline-flex items-center gap-2 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none rounded-0 text-text-secondary transition-opacity duration-150 ease-out text-sm font-medium list-none [&::-webkit-details-marker]:hidden hover:opacity-80">
                    <span className="w-4 h-4 inline-flex items-center justify-center shrink-0 text-[13px] text-text-secondary">
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
                    <span className={chevronIcon}>
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
                  <div
                    className={cn(
                      expandableBody,
                      'overflow-wrap-anywhere text-sm leading-relaxed text-text-secondary prose-chat'
                    )}
                  >
                    <MarkdownContent text={block} defer={streaming} />
                  </div>
                </details>
              ))
            : null}
          {!useThinkingChrome && inlineReasoningText ? (
            <div className="mb-3 overflow-wrap-anywhere text-sm leading-relaxed text-text-secondary prose-chat">
              <MarkdownContent text={inlineReasoningText} defer={streaming} />
            </div>
          ) : null}
          {visibleText ? <MarkdownContent text={visibleText} defer={streaming} /> : null}
          {message.streaming && (
            <span className="ml-px inline-block animate-blink text-text-secondary motion-reduce:animate-none">
              ▋
            </span>
          )}
        </div>
        {metrics && useThinkingChrome && (
          <div className="mt-3 border-t border-border pt-2 text-xs leading-relaxed text-text-secondary">
            📊 {formatTokenCount(metrics.estimatedTokens)} tokens
            {getCacheIndicator(metrics)}
          </div>
        )}
      </div>
    </div>
  );
}

export default memo(AssistantMessage);
