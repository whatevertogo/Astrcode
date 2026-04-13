import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  composerAttachmentButton,
  composerInterruptButton,
  composerShell,
  composerSubmitButton,
} from '../../lib/styles';
import type { ComposerOption } from '../../types';
import CommandSelector from './CommandSelector';
import { useChatScreenContext } from './ChatScreenContext';
import ModelSelector from './ModelSelector';
import { logger } from '../../lib/logger';

/**
 * 输入框组件，支持 '/' 触发 slash 候选选择器
 *
 * 当用户在输入框中输入 '/' 时（行首或空格后），会弹出候选面板。
 * 面板展示当前会话可用的 command / skill 列表，支持：
 * - 键盘 ↑↓ 导航、Enter 确认选中、Escape 关闭
 * - 模糊搜索匹配（title / description / keywords）
 * - 选中后将 insertText 替换 '/' 前缀并写回输入框
 */

export default function InputBar() {
  const {
    sessionId,
    workingDir,
    phase,
    onSubmitPrompt,
    onInterrupt,
    listComposerOptions,
    modelRefreshKey,
    getCurrentModel,
    listAvailableModels,
    setModel,
  } = useChatScreenContext();
  const [value, setValue] = useState('');
  const [isComposing, setIsComposing] = useState(false);
  const [slashTriggerVisible, setSlashTriggerVisible] = useState(false);
  const [slashQuery, setSlashQuery] = useState('');
  const [slashOptions, setSlashOptions] = useState<ComposerOption[]>([]);
  const [slashLoading, setSlashLoading] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const slashTriggerStartRef = useRef(0);
  const slashTriggerEndRef = useRef(0);
  const slashAbortRef = useRef<AbortController | null>(null);
  const isBusy = phase !== 'idle';

  const closeSlashTrigger = useCallback(() => {
    setSlashTriggerVisible(false);
    setSlashQuery('');
    setSlashOptions([]);
    setSlashLoading(false);
    slashAbortRef.current?.abort();
    slashAbortRef.current = null;
  }, []);

  /**
   * 检测光标位置是否处于 '/' 触发上下文
   * @returns {triggerStart: '/' 的字符索引, triggerEnd: 光标位置, query: '/' 后的文本}
   */
  function findSlashTrigger(
    currentValue: string,
    cursorPos: number
  ): { triggerStart: number; triggerEnd: number; query: string } | null {
    const lineStart = Math.max(0, currentValue.lastIndexOf('\n', cursorPos - 1) + 1);
    const segment = currentValue.slice(lineStart, cursorPos);
    const slashIdx = segment.lastIndexOf('/');
    if (slashIdx === -1) {
      return null;
    }

    const beforeSlash = slashIdx === 0 ? '' : segment[slashIdx - 1];
    if (beforeSlash !== ' ' && slashIdx !== 0) {
      return null;
    }

    const afterSlash = segment.slice(slashIdx + 1);
    if (/\s/.test(afterSlash)) {
      return null;
    }

    return { triggerStart: lineStart + slashIdx, triggerEnd: cursorPos, query: afterSlash };
  }

  useEffect(() => {
    if (!sessionId) {
      closeSlashTrigger();
      return;
    }

    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }

    const trigger = findSlashTrigger(value, textarea.selectionStart);
    if (trigger) {
      slashTriggerStartRef.current = trigger.triggerStart;
      slashTriggerEndRef.current = trigger.triggerEnd;
      setSlashQuery(trigger.query);
      if (!slashTriggerVisible) {
        setSlashTriggerVisible(true);
      }
      return;
    }

    if (slashTriggerVisible) {
      closeSlashTrigger();
    }
  }, [closeSlashTrigger, sessionId, slashTriggerVisible, value]);

  useEffect(() => {
    if (!slashTriggerVisible || !sessionId) {
      return;
    }

    slashAbortRef.current?.abort();
    const controller = new AbortController();
    slashAbortRef.current = controller;
    setSlashLoading(true);

    listComposerOptions(sessionId, slashQuery, controller.signal)
      .then((options) => {
        if (controller.signal.aborted) {
          return;
        }
        setSlashOptions(options);
        setSlashLoading(false);
      })
      .catch((error) => {
        if (controller.signal.aborted) {
          return;
        }
        logger.warn('InputBar', '[CommandSelector] 获取技能选项失败:', error);
        setSlashOptions([]);
        setSlashLoading(false);
      });

    return () => {
      controller.abort();
    };
  }, [listComposerOptions, sessionId, slashQuery, slashTriggerVisible]);

  const handleComposerOptionSelect = useCallback(
    (option: ComposerOption) => {
      const before = value.slice(0, slashTriggerStartRef.current);
      const after = value.slice(slashTriggerEndRef.current);
      const insertText = option.insertText;
      const nextValue = `${before}${insertText} ${after}`;
      setValue(nextValue);
      closeSlashTrigger();

      requestAnimationFrame(() => {
        const textarea = textareaRef.current;
        if (!textarea) {
          return;
        }
        const nextCursor = before.length + insertText.length + 1;
        textarea.focus();
        textarea.setSelectionRange(nextCursor, nextCursor);
        textarea.style.height = 'auto';
        textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
      });
    },
    [closeSlashTrigger, value]
  );

  const handleInput = (event: React.ChangeEvent<HTMLTextAreaElement>) => {
    setValue(event.target.value);
    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }
    textarea.style.height = 'auto';
    textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
  };

  const submit = () => {
    const trimmed = value.trim();
    const isSlashCommand = trimmed.startsWith('/');
    if (!trimmed || (isBusy && !isSlashCommand)) {
      return;
    }
    closeSlashTrigger();
    void onSubmitPrompt(trimmed);
    setValue('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (slashTriggerVisible) {
      switch (event.key) {
        case 'Escape':
          event.preventDefault();
          closeSlashTrigger();
          return;
        case 'ArrowUp':
        case 'ArrowDown':
          event.preventDefault();
          return;
      }
    }

    if (event.key === 'Enter' && !event.shiftKey && !isComposing) {
      event.preventDefault();
      submit();
    }
  };

  return (
    <div className="max-sm:py-3.5 flex-shrink-0 bg-panel-bg px-[var(--chat-content-horizontal-padding)] pb-[18px] pt-4 max-sm:px-[var(--chat-content-horizontal-padding-mobile)]">
      <div className="mx-auto w-full max-w-[var(--chat-composer-max-width)] translate-x-[var(--chat-assistant-center-shift)] max-sm:translate-x-0">
        <div className="relative w-full">
          <div className={composerShell}>
            {workingDir && (
              <div
                className="flex items-center gap-2 rounded-t-[23px] border-b border-border bg-white/40 px-4 py-2.5 text-text-secondary"
                title={workingDir}
              >
                <span
                  className="inline-flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center"
                  aria-hidden="true"
                >
                  <svg className="h-3.5 w-3.5" viewBox="0 0 20 20">
                    <path
                      d="M2.5 5.75A1.75 1.75 0 0 1 4.25 4h4.03c.46 0 .9.18 1.23.5l1.02 1c.32.3.74.47 1.18.47h4.04A1.75 1.75 0 0 1 17.5 7.72v6.53A1.75 1.75 0 0 1 15.75 16H4.25A1.75 1.75 0 0 1 2.5 14.25V5.75Z"
                      fill="none"
                      stroke="currentColor"
                      strokeLinejoin="round"
                      strokeWidth="1.4"
                    />
                  </svg>
                </span>
                <div className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs">
                  {workingDir}
                </div>
              </div>
            )}
            <div className="relative">
              <div className="flex flex-col px-[var(--chat-composer-shell-padding-x)] py-3">
                <textarea
                  ref={textareaRef}
                  className="mb-3 max-h-[240px] min-h-[50px] w-full resize-none overflow-y-auto border-0 bg-transparent p-0 text-[15px] leading-[1.75] text-text-primary placeholder:text-text-muted focus:outline-none disabled:cursor-not-allowed disabled:opacity-60"
                  placeholder="向 AstrCode 提问..."
                  value={value}
                  rows={1}
                  onChange={handleInput}
                  onKeyDown={handleKeyDown}
                  onCompositionStart={() => setIsComposing(true)}
                  onCompositionEnd={() => setIsComposing(false)}
                />
                <div className="flex items-center justify-between">
                  <div className="flex flex-shrink-0 items-center gap-2">
                    {/* TODO: 后续在这里接入附件 / 扩展入口，当前先显式呈现为不可用占位态。 */}
                    <button
                      type="button"
                      className={composerAttachmentButton}
                      disabled
                      aria-disabled="true"
                      title="附件功能待实现"
                    >
                      <svg
                        width="18"
                        height="18"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        aria-hidden="true"
                      >
                        <line x1="12" y1="5" x2="12" y2="19"></line>
                        <line x1="5" y1="12" x2="19" y2="12"></line>
                      </svg>
                    </button>
                    <ModelSelector
                      refreshKey={modelRefreshKey}
                      getCurrentModel={getCurrentModel}
                      listAvailableModels={listAvailableModels}
                      setModel={setModel}
                    />
                  </div>
                  <div className="flex flex-shrink-0 items-center gap-2">
                    {isBusy ? (
                      <button
                        className={composerInterruptButton}
                        type="button"
                        onClick={() => void onInterrupt()}
                      >
                        中断
                      </button>
                    ) : (
                      <button
                        className={composerSubmitButton}
                        type="button"
                        onClick={submit}
                        disabled={!value.trim()}
                        aria-label="发送消息"
                        title="发送消息"
                      >
                        <svg
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2.5"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          aria-hidden="true"
                        >
                          <line x1="12" y1="19" x2="12" y2="5"></line>
                          <polyline points="5 12 12 5 19 12"></polyline>
                        </svg>
                      </button>
                    )}
                  </div>
                </div>
              </div>
            </div>
          </div>
          {sessionId && slashTriggerVisible && (
            <CommandSelector
              visible={slashTriggerVisible}
              options={slashOptions}
              loading={slashLoading}
              query={slashQuery}
              onSelect={handleComposerOptionSelect}
              onClose={closeSlashTrigger}
            />
          )}
        </div>
      </div>
      <div className="mx-auto mt-2.5 w-full max-w-[var(--chat-composer-max-width)] text-center text-xs text-text-muted">
        AI 可能会产生误导性信息，请核实重要内容
      </div>
    </div>
  );
}
