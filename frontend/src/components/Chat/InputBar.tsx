import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { ComposerOption, Phase, CurrentModelInfo, ModelOption } from '../../types';
import CommandSelector from './CommandSelector';
import ModelSelector from './ModelSelector';

/**
 * 输入框组件，支持 '/' 触发技能选择器
 *
 * 当用户在输入框中输入 '/' 时（行首或空格后），会弹出技能选择面板。
 * 面板展示当前会话可用的 skill/capability 列表，支持：
 * - 键盘 ↑↓ 导航、Enter 确认选中、Escape 关闭
 * - 模糊搜索匹配（title / description / keywords）
 * - 选中后将 insertText 替换 '/' 前缀并写回输入框
 */

interface InputBarProps {
  /** 当前会话 ID，用于拉取技能候选 */
  sessionId: string | null;
  workingDir: string;
  phase: Phase;
  onSubmit: (text: string) => void | Promise<void>;
  onInterrupt: () => void | Promise<void>;
  listComposerOptions: (
    sessionId: string,
    query: string,
    signal?: AbortSignal
  ) => Promise<ComposerOption[]>;
  modelRefreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

export default function InputBar({
  sessionId,
  workingDir,
  phase,
  onSubmit,
  onInterrupt,
  listComposerOptions,
  modelRefreshKey,
  getCurrentModel,
  listAvailableModels,
  setModel,
}: InputBarProps) {
  const [value, setValue] = useState('');
  const [isComposing, setIsComposing] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const isBusy = phase !== 'idle';

  // Skill Selector 状态
  const [slashTriggerVisible, setSlashTriggerVisible] = useState(false);
  const [slashQuery, setSlashQuery] = useState('');
  const [slashOptions, setSlashOptions] = useState<ComposerOption[]>([]);
  const [slashLoading, setSlashLoading] = useState(false);
  // 记录 '/' 触发位置，用于选中后替换
  const slashTriggerStartRef = useRef(0);
  const slashTriggerEndRef = useRef(0);

  // AbortController 引用，避免会话切换后旧请求更新状态
  const slashAbortRef = useRef<AbortController | null>(null);

  // 关闭选择器
  const closeSlashTrigger = useCallback(() => {
    setSlashTriggerVisible(false);
    setSlashQuery('');
    setSlashOptions([]);
    setSlashLoading(false);
    // 停止正在进行的请求
    slashAbortRef.current?.abort();
    slashAbortRef.current = null;
  }, []);

  /**
   * 检测光标位置是否处于 '/' 触发上下文
   * @returns {triggerStart: '/' 的字符索引, triggerEnd: 光标位置, query: '/' 后的文本}
   */
  function findSlashTrigger(
    val: string,
    cursorPos: number
  ): { triggerStart: number; triggerEnd: number; query: string } | null {
    const lineStart = Math.max(0, val.lastIndexOf('\n', cursorPos - 1) + 1);
    const segment = val.slice(lineStart, cursorPos);

    const slashIdx = segment.lastIndexOf('/');
    if (slashIdx === -1) return null;

    // '/' 前必须是行首或空格
    const beforeSlash = slashIdx === 0 ? '' : segment[slashIdx - 1];
    if (beforeSlash !== ' ' && slashIdx !== 0) return null;

    const afterSlash = segment.slice(slashIdx + 1);
    // '/' 后不能有空格（否则不是命令前缀）
    if (/\s/.test(afterSlash)) return null;

    return { triggerStart: lineStart + slashIdx, triggerEnd: cursorPos, query: afterSlash };
  }

  // 当输入变化时检测 '/' 触发
  useEffect(() => {
    if (!sessionId) {
      closeSlashTrigger();
      return;
    }

    const textarea = textareaRef.current;
    if (!textarea) return;

    const cursorPos = textarea.selectionStart;
    const trigger = findSlashTrigger(value, cursorPos);

    if (trigger) {
      slashTriggerStartRef.current = trigger.triggerStart;
      slashTriggerEndRef.current = trigger.triggerEnd;
      setSlashQuery(trigger.query);
      if (!slashTriggerVisible) setSlashTriggerVisible(true);
    } else if (slashTriggerVisible) {
      closeSlashTrigger();
    }
  }, [value, sessionId, slashTriggerVisible, closeSlashTrigger]);

  // 当 slashQuery 变化时拉取候选项
  useEffect(() => {
    if (!slashTriggerVisible || !sessionId) return;

    // 取消旧请求
    slashAbortRef.current?.abort();
    const controller = new AbortController();
    slashAbortRef.current = controller;

    setSlashLoading(true);
    listComposerOptions(sessionId, slashQuery, controller.signal)
      .then((options) => {
        if (!controller.signal.aborted) {
          setSlashOptions(options);
          setSlashLoading(false);
        }
      })
      .catch((err) => {
        if (!controller.signal.aborted) {
          console.warn('[CommandSelector] 获取技能选项失败:', err);
          setSlashOptions([]);
          setSlashLoading(false);
        }
      });

    return () => {
      controller.abort();
    };
  }, [slashQuery, slashTriggerVisible, sessionId, listComposerOptions]);

  /**
   * 选中某个技能后，将 insertText 替换掉 '/' 前缀并写回输入框
   */
  const handleSkillSelect = useCallback(
    (option: ComposerOption) => {
      const before = value.slice(0, slashTriggerStartRef.current);
      const after = value.slice(slashTriggerEndRef.current);
      // 后端 insert_text 格式为 /skill-id（如 /git-commit）
      // 选中后保留 / 前缀并将光标置于插入文本之后
      const insertText = option.insertText;
      const newValue = before + insertText + ' ' + after;
      setValue(newValue);
      closeSlashTrigger();

      // 让 textarea 获得焦点并将光标置于插入文本之后
      requestAnimationFrame(() => {
        const ta = textareaRef.current;
        if (ta) {
          const newPos = before.length + insertText.length + 1;
          ta.focus();
          ta.setSelectionRange(newPos, newPos);
          // 触发 auto-resize
          ta.style.height = 'auto';
          ta.style.height = `${Math.min(ta.scrollHeight, 200)}px`;
        }
      });
    },
    [value, closeSlashTrigger]
  );

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
    // 提交前关闭技能选择器
    closeSlashTrigger();
    void onSubmit(trimmed);
    setValue('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // 技能选择器可见时，让选择器处理 ↑↓ / Enter / Escape
    if (slashTriggerVisible) {
      switch (e.key) {
        case 'Escape':
          e.preventDefault();
          closeSlashTrigger();
          return;
        // ArrowUp/ArrowDown/Enter 交由 CommandSelector 的全局键盘监听处理
        // 但需要阻止默认行为（避免 Enter 提交、arrow 移动光标）
        case 'ArrowUp':
        case 'ArrowDown':
          e.preventDefault();
          return;
      }
    }

    if (e.key === 'Enter' && !e.shiftKey && !isComposing) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="px-8 pt-4 pb-[18px] bg-[var(--panel-bg)] flex-shrink-0">
      <div className="relative w-full max-w-[860px] mx-auto">
        <div className="bg-[linear-gradient(180deg,rgba(255,255,255,0.96)_0%,rgba(253,248,241,0.98)_100%)] border border-[rgba(230,220,205,0.95)] rounded-[24px] shadow-[0_24px_42px_rgba(117,90,52,0.1),inset_0_1px_0_rgba(255,255,255,0.82)] transition-[border-color,box-shadow,transform] duration-[180ms] ease-out focus-within:border-[rgba(122,185,153,0.56)] focus-within:shadow-[0_0_0_4px_rgba(57,201,143,0.12),0_28px_48px_rgba(117,90,52,0.13)] focus-within:-translate-y-px">
          {workingDir && (
            <div
              className="flex items-center gap-2 px-4 py-2.5 border-b border-[var(--border)] text-[var(--text-secondary)] bg-white/40 rounded-t-[23px]"
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
          <div className="relative">
            <div className="flex flex-col px-4 py-3">
              <textarea
                ref={textareaRef}
                className="w-full min-h-[50px] max-h-[240px] text-[var(--text-primary)] text-[15px] leading-[1.75] overflow-y-auto placeholder:text-[var(--text-muted)] disabled:opacity-60 disabled:cursor-not-allowed border-0 bg-transparent focus:outline-none resize-none p-0 mb-3"
                placeholder="向 AstrCode 提问..."
                value={value}
                disabled={isBusy}
                rows={1}
                onChange={handleInput}
                onKeyDown={handleKeyDown}
                onCompositionStart={() => setIsComposing(true)}
                onCompositionEnd={() => setIsComposing(false)}
              />
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 flex-shrink-0">
                  <button
                    type="button"
                    className="w-8 h-8 flex items-center justify-center text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded-full transition-colors"
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
                <div className="flex items-center gap-2 flex-shrink-0">
                  {isBusy ? (
                    <button
                      className="h-9 px-3.5 bg-[var(--danger-soft)] text-[var(--danger)] border border-[#f2d2cc] rounded-xl text-[13px] font-semibold flex-shrink-0 transition-colors duration-150 ease-out hover:bg-[#ffe7e2]"
                      type="button"
                      onClick={() => void onInterrupt()}
                    >
                      中断
                    </button>
                  ) : (
                    <button
                      className="w-9 h-9 inline-flex items-center justify-center bg-gradient-to-b from-[#35302b] to-[#26211d] text-white rounded-full flex-shrink-0 transition-[transform,background-color,opacity,box-shadow] duration-150 ease-out shadow-[0_14px_26px_rgba(47,43,39,0.16)] hover:from-[#2f2b27] hover:to-[#1f1b17] hover:-translate-y-px hover:scale-105 hover:shadow-[0_18px_32px_rgba(47,43,39,0.2)] focus-visible:outline-none focus-visible:shadow-[0_0_0_4px_rgba(57,201,143,0.16),0_18px_32px_rgba(47,43,39,0.2)] disabled:opacity-35 disabled:cursor-not-allowed [&_svg]:w-[16px] [&_svg]:h-[16px]"
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
        {/* Skill Selector 悬浮面板，与整个输入框同级，在 relative 定位上下文中 浮动 */}
        {sessionId && slashTriggerVisible && (
          <CommandSelector
            visible={slashTriggerVisible}
            options={slashOptions}
            loading={slashLoading}
            query={slashQuery}
            onSelect={handleSkillSelect}
            onClose={closeSlashTrigger}
          />
        )}
      </div>
      <div className="w-full max-w-[860px] mx-auto mt-2.5 text-center text-xs text-[var(--text-muted)]">
        AI 可能会产生误导性信息，请核实重要内容
      </div>
    </div>
  );
}
