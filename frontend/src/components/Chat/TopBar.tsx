import React from 'react';
import styles from './TopBar.module.css';

interface TopBarProps {
  projectName: string | null;
  sessionTitle: string | null;
  onNewSession: () => void;
}

export default function TopBar({ projectName, sessionTitle, onNewSession }: TopBarProps) {
  return (
    <div className={styles.topBar}>
      <div className={styles.breadcrumb}>
        {projectName ? (
          <>
            <span className={styles.projectName}>{projectName}</span>
            {sessionTitle && (
              <>
                <span className={styles.sep}>›</span>
                <span className={styles.sessionTitle}>{sessionTitle}</span>
              </>
            )}
          </>
        ) : (
          <span className={styles.empty}>未选择会话</span>
        )}
      </div>
      <button className={styles.newSessionBtn} onClick={onNewSession}>
        + 新会话
      </button>
    </div>
  );
}
