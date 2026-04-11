import React, { useEffect, useState } from 'react';
import {
  btnPrimary,
  btnSecondary,
  dialogSurface,
  fieldButton,
  fieldInput,
  overlay,
} from '../lib/styles';
import { cn } from '../lib/utils';

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
      className={overlay}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onCancel();
        }
      }}
    >
      <div className={cn(dialogSurface, 'w-[400px] max-w-full')} onKeyDown={handleKeyDown}>
        <div className="text-lg font-bold text-text-primary mb-5">新建项目</div>
        <div className="mb-4">
          <label className="block text-[13px] text-text-secondary mb-2 font-semibold">
            工作目录
          </label>
          <input
            className={fieldInput}
            placeholder="输入完整目录路径"
            value={workingDir}
            onChange={(event) => setWorkingDir(event.target.value)}
            autoFocus
          />
        </div>
        <div className="mb-4">
          <button
            type="button"
            className={fieldButton}
            onClick={() => void handleChooseDirectory()}
            disabled={!canSelectDirectory}
          >
            <span className="flex-1 min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-left text-text-primary">
              {canSelectDirectory ? '浏览并选择文件夹' : '浏览仅桌面端可用'}
            </span>
            <span className="shrink-0 font-semibold text-text-secondary">浏览</span>
          </button>
        </div>
        <div className="flex justify-end gap-2.5 mt-6">
          <button className={btnSecondary} onClick={onCancel}>
            取消
          </button>
          <button className={btnPrimary} onClick={handleConfirm} disabled={!workingDir.trim()}>
            确认
          </button>
        </div>
      </div>
    </div>
  );
}
