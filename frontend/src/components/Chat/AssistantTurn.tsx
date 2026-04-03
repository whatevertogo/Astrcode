import React, { memo } from 'react';
import type { Message } from '../../types';
import AssistantMessage from './AssistantMessage';
import ToolCallBlock from './ToolCallBlock';
import styles from './AssistantMessage.module.css';

interface AssistantTurnProps {
  messages: Message[];
}

function AssistantTurn({ messages }: AssistantTurnProps) {
  if (messages.length === 0) return null;

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
        {messages.map((msg) => {
          if (msg.kind === 'assistant') {
            return <AssistantMessage key={msg.id} message={msg} />;
          }
          if (msg.kind === 'toolCall') {
            return <ToolCallBlock key={msg.id} message={msg} />;
          }
          return null;
        })}
      </div>
    </div>
  );
}

export default memo(AssistantTurn);
