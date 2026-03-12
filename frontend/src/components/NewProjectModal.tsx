import React, { useEffect, useState } from 'react';
import styles from './NewProjectModal.module.css';

interface NewProjectModalProps {
  canSelectDirectory: boolean;
  defaultWorkingDir?: string;
  onSelectDirectory: () => Promise<string | null>;
  onConfirm: (workingDir: string) => void;
  onCancel: () => void;
}

export default function NewProjectModal({
  canSelectDirectory,
  defaultWorkingDir = '',
  onSelectDirectory,
  onConfirm,
  onCancel,
}: NewProjectModalProps) {
  const [workingDir, setWorkingDir] = useState(defaultWorkingDir);

  useEffect(() => {
    setWorkingDir(defaultWorkingDir);
  }, [defaultWorkingDir]);

  const handleChooseDirectory = async () => {
    const selected = await onSelectDirectory();
    if (selected) {
      setWorkingDir(selected);
    }
  };

  const handleConfirm = () => {
    const trimmed = workingDir.trim();
    if (!trimmed) {
      return;
    }
    onConfirm(trimmed);
  };

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Enter') {
      handleConfirm();
    }
    if (event.key === 'Escape') {
      onCancel();
    }
  };

  return (
    <div
      className={styles.overlay}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onCancel();
        }
      }}
    >
      <div className={styles.modal} onKeyDown={handleKeyDown}>
        <div className={styles.title}>新建项目</div>
        <div className={styles.field}>
          <label className={styles.label}>工作目录</label>
          <input
            className={styles.input}
            placeholder="输入完整目录路径"
            value={workingDir}
            onChange={(event) => setWorkingDir(event.target.value)}
            autoFocus
          />
        </div>
        <div className={styles.field}>
          <button
            type="button"
            className={styles.pathPicker}
            onClick={() => void handleChooseDirectory()}
            disabled={!canSelectDirectory}
          >
            <span className={styles.pathValue}>
              {canSelectDirectory ? '浏览并选择文件夹' : '浏览仅桌面端可用'}
            </span>
            <span className={styles.pathAction}>浏览</span>
          </button>
        </div>
        <div className={styles.actions}>
          <button className={styles.cancelBtn} onClick={onCancel}>
            取消
          </button>
          <button
            className={styles.confirmBtn}
            onClick={handleConfirm}
            disabled={!workingDir.trim()}
          >
            确认
          </button>
        </div>
      </div>
    </div>
  );
}
