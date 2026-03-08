import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { AssistantMessage as AssistantMessageType } from '../../types';
import styles from './AssistantMessage.module.css';

interface AssistantMessageProps {
  message: AssistantMessageType;
}

export default function AssistantMessage({ message }: AssistantMessageProps) {
  return (
    <div className={styles.wrapper}>
      <div className={styles.bubble}>
        <div className={styles.label}>Assistant</div>
        <div className={styles.content}>
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
          {message.streaming && <span className={styles.cursor}>▋</span>}
        </div>
      </div>
    </div>
  );
}
