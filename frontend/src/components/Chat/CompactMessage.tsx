import { memo } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { CompactMessage as CompactMessageType } from '../../types';
import styles from './CompactMessage.module.css';

interface CompactMessageProps {
  message: CompactMessageType;
}

function CompactMessage({ message }: CompactMessageProps) {
  const triggerLabel = message.trigger === 'manual' ? '手动压缩' : '自动压缩';

  return (
    <div className={styles.wrapper}>
      <div className={styles.badgeRow}>
        <span className={styles.badge}>{triggerLabel}</span>
        <span className={styles.meta}>保留最近 {message.preservedRecentTurns} 个 turn</span>
      </div>
      <div className={styles.body}>
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.summary}</ReactMarkdown>
      </div>
    </div>
  );
}

export default memo(CompactMessage);
