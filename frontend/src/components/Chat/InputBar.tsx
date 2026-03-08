import React, { useRef, useState } from 'react';
import type { Phase } from '../../types';
import styles from './InputBar.module.css';

interface InputBarProps {
  workingDir: string;
  phase: Phase;
  onSubmit: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
}

export default function InputBar({
  workingDir,
  phase,
  onSubmit,
  onInterrupt,
}: InputBarProps) {
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
    onSubmit(trimmed);
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
    <div className={styles.inputBar}>
      {workingDir && (
        <div className={styles.workingDir} title={workingDir}>
          {workingDir}
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
          <button className={styles.interruptBtn} onClick={onInterrupt}>
            中断
          </button>
        ) : (
          <button
            className={styles.sendBtn}
            onClick={submit}
            disabled={!value.trim()}
          >
            发送
          </button>
        )}
      </div>
    </div>
  );
}
