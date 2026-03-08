import React from 'react';
import type { UserMessage as UserMessageType } from '../../types';
import styles from './UserMessage.module.css';

interface UserMessageProps {
  message: UserMessageType;
}

export default function UserMessage({ message }: UserMessageProps) {
  return (
    <div className={styles.wrapper}>
      <div className={styles.bubble}>
        <div className={styles.label}>你</div>
        <div className={styles.text}>{message.text}</div>
      </div>
    </div>
  );
}
