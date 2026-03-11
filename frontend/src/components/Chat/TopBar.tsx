import React from 'react';
import type { CurrentModelInfo, ModelOption } from '../../types';
import ModelSelector from './ModelSelector';
import styles from './TopBar.module.css';

interface TopBarProps {
  projectName: string | null;
  sessionTitle: string | null;
  onNewSession: () => void;
  modelRefreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

export default function TopBar({
  projectName,
  sessionTitle,
  onNewSession,
  modelRefreshKey,
  getCurrentModel,
  listAvailableModels,
  setModel,
}: TopBarProps) {
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
      <ModelSelector
        refreshKey={modelRefreshKey}
        getCurrentModel={getCurrentModel}
        listAvailableModels={listAvailableModels}
        setModel={setModel}
      />
      <button className={styles.newSessionBtn} onClick={onNewSession} disabled={!projectName}>
        + 新会话
      </button>
    </div>
  );
}
