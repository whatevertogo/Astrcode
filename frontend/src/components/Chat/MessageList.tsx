import React, { useEffect, useRef } from 'react';
import type { Message } from '../../types';
import UserMessage from './UserMessage';
import AssistantMessage from './AssistantMessage';
import ToolCallBlock from './ToolCallBlock';
import styles from './MessageList.module.css';

interface MessageListProps {
  messages: Message[];
}

export default function MessageList({ messages }: MessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  return (
    <div className={styles.list}>
      {messages.length === 0 && (
        <div className={styles.empty}>向 AstrCode 提问，开始对话...</div>
      )}
      {messages.map((msg) => {
        if (msg.kind === 'user') {
          return <UserMessage key={msg.id} message={msg} />;
        }
        if (msg.kind === 'assistant') {
          return <AssistantMessage key={msg.id} message={msg} />;
        }
        if (msg.kind === 'toolCall') {
          return <ToolCallBlock key={msg.id} message={msg} />;
        }
        return null;
      })}
      <div ref={bottomRef} />
    </div>
  );
}
