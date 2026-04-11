//! # 确认对话框组件
//!
//! 替代原生 window.confirm()，保持桌面端风格一致。

import React, { memo } from 'react';
import { overlay } from '../lib/styles';
import { cn } from '../lib/utils';

interface ConfirmDialogProps {
  /** 对话框标题 */
  title: string;
  /** 提示内容 */
  message: string;
  /** 危险操作时使用 danger 样式 */
  danger?: boolean;
  /** 确认按钮文案，默认"确定" */
  confirmLabel?: string;
  /** 取消按钮文案，默认"取消" */
  cancelLabel?: string;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
}

function ConfirmDialog({
  title,
  message,
  danger = false,
  confirmLabel = '确定',
  cancelLabel = '取消',
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  // 支持 Enter / Escape 快捷键
  const handleKeyDown = React.useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        void onConfirm();
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        onCancel();
      }
    },
    [onConfirm, onCancel]
  );

  return (
    <div
      className={overlay}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={handleKeyDown}
    >
      <div className="w-[380px] max-w-full p-6 rounded-[20px] border border-border bg-surface shadow-[0_24px_60px_rgba(83,61,31,0.16)]">
        <div className="text-base font-bold text-text-primary mb-2">{title}</div>
        <div className="text-sm leading-relaxed text-text-secondary mb-5">{message}</div>
        <div className="flex justify-end gap-2.5">
          <button
            className="px-4 py-[9px] text-[13px] font-semibold rounded-xl border border-border bg-surface-soft text-text-secondary transition-[background-color,border-color,color] duration-150 ease-out hover:bg-white hover:border-border-strong hover:text-text-primary"
            type="button"
            onClick={onCancel}
            autoFocus
          >
            {cancelLabel}
          </button>
          <button
            className={cn(
              'px-4 py-[9px] text-[13px] font-semibold rounded-xl border-none bg-accent-strong text-white transition-[background-color,opacity] duration-150 ease-out hover:bg-[#1f1b17]',
              danger && 'bg-danger hover:bg-[#b54a4a]'
            )}
            type="button"
            onClick={() => {
              void onConfirm();
            }}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

export default memo(ConfirmDialog);
