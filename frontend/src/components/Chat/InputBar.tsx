import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { ComposerOption, Phase } from '../../types';
import SkillSelector from './SkillSelector';

/**
 * 检测当前光标位置是否处于 '/' 触发上下文。
 * 返回 '/' 之后的查询字符串；如果不在 '/' 触发状态则返回 undefined。
 */
function getSlashQuery(value: string, cursorPos: number): string | undefined {
  // 查找光标之前最近的 '/'
  // '/' 必须是行首或者前面是空格/换行才触发（避免在 URL 中间误触发）
  for (let i = cursorPos - 1; i >= 0; i--) {
    const ch = value[i];
    if (ch === ' ' || ch === '\n' || i === 0) {
      // 检查下一个字符是否为 '/'
      const slashIndex = ch === '/' ? i : i + 1;
      if (slashIndex < cursorPos && value[slashIndex] === '/') {
        // 提取 '/' 之后到光标位置的文本作为 query
        const query = value.slice(slashIndex + 1, cursorPos);
        // 如果 query 中包含空格或换行，则不在 '/' 命令上下文中
        if (/[\s\n]/.test(query)) return undefined;
        return query;
      }
      return undefined; // 前面没有找到 '/'
    }
    if (ch === '\n') break;
  }
  return undefined;
}

interface InputBarProps {
  workingDir: string;
  phase: Phase;
  onSubmit: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
  listComposerOptions: (
    sessionId: string,
    query: string,
    signal?: AbortSignal
  ) => Promise<ComposerOption[]>;
}

export default function InputBar({
  workingDir,
  phase,
  onSubmit,
  onInterrupt,
  listComposerOptions: _listComposerOptions,
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
    <div className="px-8 pt-4 pb-[18px] bg-[var(--panel-bg)] flex-shrink-0">
      <div className="w-full max-w-[860px] mx-auto bg-[linear-gradient(180deg,rgba(255,255,255,0.96)_0%,rgba(253,248,241,0.98)_100%)] border border-[rgba(230,220,205,0.95)] rounded-[24px] shadow-[0_24px_42px_rgba(117,90,52,0.1),inset_0_1px_0_rgba(255,255,255,0.82)] overflow-hidden transition-[border-color,box-shadow,transform] duration-[180ms] ease-out focus-within:border-[rgba(122,185,153,0.56)] focus-within:shadow-[0_0_0_4px_rgba(57,201,143,0.12),0_28px_48px_rgba(117,90,52,0.13)] focus-within:-translate-y-px">
        {workingDir && (
          <div
            className="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--border)] text-[var(--text-secondary)] bg-white/40"
            title={workingDir}
          >
            <span
              className="w-3.5 h-3.5 inline-flex items-center justify-center flex-shrink-0"
              aria-hidden="true"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 20 20">
                <path
                  d="M2.5 5.75A1.75 1.75 0 0 1 4.25 4h4.03c.46 0 .9.18 1.23.5l1.02 1c.32.3.74.47 1.18.47h4.04A1.75 1.75 0 0 1 17.5 7.72v6.53A1.75 1.75 0 0 1 15.75 16H4.25A1.75 1.75 0 0 1 2.5 14.25V5.75Z"
                  fill="none"
                  stroke="currentColor"
                  strokeLinejoin="round"
                  strokeWidth="1.4"
                />
              </svg>
            </span>
            <div className="overflow-hidden text-ellipsis whitespace-nowrap text-xs font-mono">
              {workingDir}
            </div>
          </div>
        )}
        <div className="flex items-end gap-3 px-4 py-3.5">
          <textarea
            ref={textareaRef}
            className="flex-1 min-h-[70px] max-h-[240px] text-[var(--text-primary)] text-[15px] leading-[1.75] overflow-y-auto placeholder:text-[var(--text-muted)] disabled:opacity-60 disabled:cursor-not-allowed border-0 bg-transparent focus:outline-none resize-none p-0"
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
              className="h-9.5 px-3.5 bg-[var(--danger-soft)] text-[var(--danger)] border border-[#f2d2cc] rounded-xl text-[13px] font-semibold flex-shrink-0 transition-colors duration-150 ease-out hover:bg-[#ffe7e2]"
              type="button"
              onClick={() => void onInterrupt()}
            >
              中断
            </button>
          ) : (
            <button
              className="w-10.5 h-10.5 inline-flex items-center justify-center bg-gradient-to-b from-[#35302b] to-[#26211d] text-white rounded-xl flex-shrink-0 transition-[transform,background-color,opacity,box-shadow] duration-150 ease-out shadow-[0_14px_26px_rgba(47,43,39,0.16)] hover:from-[#2f2b27] hover:to-[#1f1b17] hover:-translate-y-px hover:scale-105 hover:shadow-[0_18px_32px_rgba(47,43,39,0.2)] focus-visible:outline-none focus-visible:shadow-[0_0_0_4px_rgba(57,201,143,0.16),0_18px_32px_rgba(47,43,39,0.2)] disabled:opacity-35 disabled:cursor-not-allowed [&_svg]:w-[18px] [&_svg]:h-[18px]"
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
      <div className="w-full max-w-[860px] mx-auto mt-2.5 text-center text-xs text-[var(--text-muted)]">
        AI 可能会产生误导性信息，请核实重要内容
      </div>
    </div>
  );
}
