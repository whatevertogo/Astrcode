import { memo } from 'react';
import type { UserMessage as UserMessageType } from '../../types';
import styles from './UserMessage.module.css';

interface UserMessageProps {
  message: UserMessageType;
}

function UserMessage({ message }: UserMessageProps) {
  return (
    <div className={styles.wrapper}>
      <div className={styles.avatar} aria-hidden="true">
        U
      </div>
      <div className={styles.body}>
        <div className={styles.text}>{message.text}</div>
      </div>
    </div>
  );
}

export default memo(UserMessage);
