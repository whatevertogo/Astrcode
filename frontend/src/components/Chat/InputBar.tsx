import React, { useRef, useState } from 'react';
import type { Phase } from '../../types';
import styles from './InputBar.module.css';

interface InputBarProps {
  workingDir: string;
  phase: Phase;
  onSubmit: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
}

export default function InputBar({ workingDir, phase, onSubmit, onInterrupt }: InputBarProps) {
  const [value, setValue] = useState('');
  const [isComposing, setIsComposing] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const isBusy = phase !== 'idle';

  const handleInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(e.target.value);
    // Auto-resize
    const ta = textareaRef.current;
    if (ta) {
      ta.style.height = 'auto';
      ta.style.height = `${Math.min(ta.scrollHeight, 200)}px`;
    }
  };

  const submit = () => {
    const trimmed = value.trim();
    if (!trimmed || isBusy) return;
    void onSubmit(trimmed);
    setValue('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !isComposing) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className={styles.composerWrap}>
      <div className={styles.inputBar}>
        {workingDir && (
          <div className={styles.workingDirRow} title={workingDir}>
            <span className={styles.workingDirIcon} aria-hidden="true">
              <svg viewBox="0 0 20 20">
                <path
                  d="M2.5 5.75A1.75 1.75 0 0 1 4.25 4h4.03c.46 0 .9.18 1.23.5l1.02 1c.32.3.74.47 1.18.47h4.04A1.75 1.75 0 0 1 17.5 7.72v6.53A1.75 1.75 0 0 1 15.75 16H4.25A1.75 1.75 0 0 1 2.5 14.25V5.75Z"
                  fill="none"
                  stroke="currentColor"
                  strokeLinejoin="round"
                  strokeWidth="1.4"
                />
              </svg>
            </span>
            <div className={styles.workingDir}>{workingDir}</div>
          </div>
        )}
        <div className={styles.row}>
          <textarea
            ref={textareaRef}
            className={styles.textarea}
            placeholder="向 AstrCode 提问..."
            value={value}
            disabled={isBusy}
            rows={1}
            onChange={handleInput}
            onKeyDown={handleKeyDown}
            onCompositionStart={() => setIsComposing(true)}
            onCompositionEnd={() => setIsComposing(false)}
          />
          {isBusy ? (
            <button
              className={styles.interruptBtn}
              type="button"
              onClick={() => void onInterrupt()}
            >
              中断
            </button>
          ) : (
            <button
              className={styles.sendBtn}
              type="button"
              onClick={submit}
              disabled={!value.trim()}
              aria-label="发送消息"
              title="发送消息"
            >
              <svg viewBox="0 0 20 20" aria-hidden="true">
                <path
                  d="M4 10.5 15.3 4.8c.47-.24 1 .17.89.68l-1.73 8.24a.72.72 0 0 1-1.07.47L10 12.1 7.54 14.6a.72.72 0 0 1-1.23-.48V11.4L4.22 11a.53.53 0 0 1-.22-1Z"
                  fill="currentColor"
                />
              </svg>
            </button>
          )}
        </div>
      </div>
      <div className={styles.disclaimer}>AI 可能会产生误导性信息，请核实重要内容</div>
    </div>
  );
}
