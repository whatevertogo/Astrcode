import React, { memo, useEffect, useRef, useState } from 'react';
import styles from './ThinkingBlock.module.css';

interface ThinkingBlockProps {
  reasoningText: string;
  streaming: boolean;
}

function buildPreview(text: string): string {
  const firstNonEmptyLine = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!firstNonEmptyLine) {
    return '';
  }

  const normalized = firstNonEmptyLine.replace(/\s+/g, ' ');
  return normalized.length > 40 ? `${normalized.slice(0, 40)}...` : normalized;
}

function ThinkingBlock({ reasoningText, streaming }: ThinkingBlockProps) {
  const [expanded, setExpanded] = useState(true);
  const bodyRef = useRef<HTMLDivElement>(null);
  const preview = buildPreview(reasoningText);

  useEffect(() => {
    if (!expanded || !streaming || !bodyRef.current) {
      return;
    }

    bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
  }, [expanded, reasoningText, streaming]);

  return (
    <div className={styles.block}>
      <button
        type="button"
        className={styles.header}
        onClick={() => setExpanded((value) => !value)}
      >
        <span className={`${styles.icon} ${streaming ? styles.iconStreaming : ''}`}>◉</span>
        <span className={styles.title}>{streaming ? '思考中...' : '已思考'}</span>
        {!expanded && preview ? <span className={styles.preview}>{preview}</span> : null}
        {!streaming ? (
          <span
            className={`${styles.chevron} ${expanded ? styles.chevronExpanded : ''}`}
            aria-hidden="true"
          >
            ▾
          </span>
        ) : null}
      </button>
      <div className={`${styles.body} ${expanded ? styles.bodyExpanded : styles.bodyCollapsed}`}>
        <div ref={bodyRef} className={styles.scrollArea}>
          {reasoningText}
        </div>
      </div>
    </div>
  );
}

export default memo(ThinkingBlock);
