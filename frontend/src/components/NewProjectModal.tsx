import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import styles from './NewProjectModal.module.css';

interface NewProjectModalProps {
  onConfirm: (name: string, workingDir: string) => void;
  onCancel: () => void;
}

export default function NewProjectModal({ onConfirm, onCancel }: NewProjectModalProps) {
  const [name, setName] = useState('');
  const [workingDir, setWorkingDir] = useState('');
  const [loadingDefaultDir, setLoadingDefaultDir] = useState(true);

  const getDirectoryName = (path: string) => {
    const normalized = path.replace(/[\\/]+$/, '');
    const parts = normalized.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] || '默认项目';
  };

  useEffect(() => {
    let cancelled = false;
    void invoke<string>('get_working_dir')
      .then((dir) => {
        if (cancelled) {
          return;
        }
        setWorkingDir(dir);
        setName((current) => current.trim() || getDirectoryName(dir));
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingDefaultDir(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const handleChooseDirectory = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: workingDir || undefined,
      title: '选择工作目录',
    });

    if (typeof selected !== 'string' || !selected) {
      return;
    }

    setWorkingDir(selected);
    setName((current) => {
      const trimmed = current.trim();
      if (!trimmed || trimmed === getDirectoryName(workingDir)) {
        return getDirectoryName(selected);
      }
      return current;
    });
  };

  const handleConfirm = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    onConfirm(trimmed, workingDir.trim());
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleConfirm();
    if (e.key === 'Escape') onCancel();
  };

  return (
    <div className={styles.overlay} onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}>
      <div className={styles.modal} onKeyDown={handleKeyDown}>
        <div className={styles.title}>新建项目</div>
        <div className={styles.field}>
          <label className={styles.label}>项目名称</label>
          <input
            className={styles.input}
            placeholder="我的项目"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </div>
        <div className={styles.field}>
          <label className={styles.label}>工作目录（可选）</label>
          <button
            type="button"
            className={styles.pathPicker}
            onClick={() => void handleChooseDirectory()}
          >
            <span className={styles.pathValue}>
              {workingDir || '点击选择文件夹'}
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
            disabled={!name.trim() || loadingDefaultDir}
          >
            确认
          </button>
        </div>
      </div>
    </div>
  );
}
