/**
 * Skill 选择器弹出面板组件
 *
 * 当用户在输入框中输入 '/' 时触发，展示当前会话可用的 slash command / 技能列表。
 * 支持键盘 ↑↓ 导航、Enter 确认选中、Escape 关闭。
 * 样式适配 AstrCode 暖色/米色主题风格。
 *
 * 后端已完整提供：
 * - API: GET /api/sessions/{id}/composer/options?kinds=skill&q={query}
 * - 类型: ComposerOptionDto (protocol) → ComposerOption (frontend)
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ComposerOption } from '../../types';
import { cn } from '../../lib/utils';

interface CommandSelectorProps {
  /** 是否可见 */
  visible: boolean;
  /** 候选技能列表 */
  options: ComposerOption[];
  /** 加载状态 */
  loading: boolean;
  /** 选择事件回调 */
  onSelect: (option: ComposerOption) => void;
  /** 关闭事件回调 */
  onClose: () => void;
  /** 查询关键词（用户输入 '/' 后的内容） */
  query: string;
}

export default function CommandSelector({
  visible,
  options,
  loading,
  onSelect,
  onClose,
  query,
}: CommandSelectorProps) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const panelRef = useRef<HTMLDivElement>(null);

  /**
   * 根据 query 在 title / description / keywords 中模糊匹配
   * 为空 query 时返回全部候选项
   */
  const filteredOptions = useMemo(() => {
    if (!query) return options;
    const q = query.toLowerCase();
    return options.filter(
      (opt) =>
        opt.title.toLowerCase().includes(q) ||
        opt.description.toLowerCase().includes(q) ||
        (opt.keywords ?? []).some((kw) => kw.toLowerCase().includes(q))
    );
  }, [options, query]);

  /** 重置选中索引到第一项 */
  const resetSelection = useCallback(() => {
    setSelectedIndex(0);
  }, []);

  /**
   * 全局键盘事件监听（仅在选择框可见时生效）
   * ArrowUp/ArrowDown: 移动选中项
   * Enter: 确认选择（需配合非 shift/非 composition 状态）
   * Escape: 关闭面板
   */
  useEffect(() => {
    if (!visible) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault();
          e.stopPropagation();
          setSelectedIndex((prev) => (prev + 1) % filteredOptions.length);
          break;
        case 'ArrowUp':
          e.preventDefault();
          e.stopPropagation();
          setSelectedIndex((prev) => (prev - 1 + filteredOptions.length) % filteredOptions.length);
          break;
        case 'Tab':
        case 'Enter':
          // 在处理 IME 中文输入法时不拦截
          if (!e.shiftKey && !e.isComposing) {
            e.preventDefault();
            e.stopPropagation();
            if (filteredOptions[selectedIndex]) {
              onSelect(filteredOptions[selectedIndex]);
            }
          }
          break;
        case 'Escape':
          e.preventDefault();
          e.stopPropagation();
          onClose();
          break;
      }
    };

    // 使用 capture: true 可以在捕获阶段（优先于输入框的冒泡阶段）拦截按键事件
    window.addEventListener('keydown', handleKeyDown, { capture: true });
    return () => window.removeEventListener('keydown', handleKeyDown, { capture: true });
  }, [visible, filteredOptions, selectedIndex, onSelect, onClose]);

  /** 每次可见性或 query 变化时重置选中项 */
  useEffect(() => {
    if (visible) resetSelection();
  }, [visible, query, resetSelection]);

  /** 滚动到当前选中的选项 */
  useEffect(() => {
    if (!visible || !filteredOptions[selectedIndex]) return;
    const target = panelRef.current?.querySelector(`[data-index="${selectedIndex}"]`);
    target?.scrollIntoView({ block: 'nearest' });
  }, [selectedIndex, visible, filteredOptions]);

  /** 不可见或无候选项时不渲染（空状态另有处理） */
  if (!visible) return null;

  return (
    <div
      className="absolute bottom-[calc(100%+8px)] left-1/2 -translate-x-1/2 w-[calc(100%-24px)] max-w-[760px] max-h-[420px] overflow-y-auto rounded-xl border border-border bg-surface shadow-2xl p-1.5 z-[9999]"
      ref={panelRef}
      onMouseDown={(e) => e.preventDefault()}
      role="listbox"
      aria-label="命令选择"
    >
      {loading ? (
        <div className="flex items-center justify-center py-4 text-xs text-text-muted">
          加载中...
        </div>
      ) : filteredOptions.length === 0 ? (
        <div className="px-3 py-2 text-xs text-text-faint">没有找到匹配「{query}」的命令</div>
      ) : (
        filteredOptions.map((option, index) => {
          const previousOption = index > 0 ? filteredOptions[index - 1] : null;
          const showHeader = !previousOption || previousOption.kind !== option.kind;
          const headerText =
            option.kind === 'command'
              ? '命令'
              : option.kind === 'skill'
                ? '技能'
                : option.kind === 'capability'
                  ? '系统能力'
                  : '命令';

          return (
            <React.Fragment key={option.id}>
              {showHeader && (
                <div className="px-3 py-1.5 mt-1 first:mt-0 text-[11px] font-semibold text-text-muted tracking-wider">
                  {headerText}
                </div>
              )}
              <button
                type="button"
                role="option"
                aria-selected={index === selectedIndex}
                data-index={index}
                onMouseEnter={() => setSelectedIndex(index)}
                onClick={() => onSelect(option)}
                className={cn(
                  'w-full flex items-center justify-start gap-3 px-2 h-[34px] text-left transition-all duration-75 rounded-lg cursor-pointer',
                  index === selectedIndex
                    ? 'bg-[rgba(0,0,0,0.06)] text-text-primary'
                    : 'text-text-secondary'
                )}
              >
                <span
                  className={cn(
                    'flex items-center justify-center shrink-0 w-4 h-4',
                    index === selectedIndex ? 'text-text-primary' : 'text-text-muted'
                  )}
                >
                  <CommandIcon kind={option.kind} isSelected={index === selectedIndex} />
                </span>
                <div className="flex flex-1 items-center gap-3 min-w-0 overflow-hidden">
                  <span
                    className={cn(
                      'text-[13px] shrink-0 text-inherit leading-normal',
                      index === selectedIndex ? 'font-semibold' : 'font-medium'
                    )}
                  >
                    {option.title}
                  </span>
                  {option.description && (
                    <span
                      className={cn(
                        'text-[12px] truncate min-w-0 flex-1 leading-normal',
                        index === selectedIndex ? 'text-text-secondary' : 'text-text-muted'
                      )}
                      title={option.description}
                    >
                      {option.description}
                    </span>
                  )}
                  {option.badges && option.badges.length > 0 && (
                    <div className="flex items-center gap-1 shrink-0 ml-auto">
                      {option.badges.map((badge) => (
                        <span
                          key={badge}
                          className="inline-block px-1.5 py-[2px] text-[10px] leading-normal border-none uppercase font-semibold tracking-wide text-text-muted"
                        >
                          {badge}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              </button>
            </React.Fragment>
          );
        })
      )}
    </div>
  );
}

/**
 * Skill / Capability 图标，根据 kind 展示不同图标
 */
function CommandIcon({ kind, isSelected }: { kind: ComposerOption['kind']; isSelected: boolean }) {
  const strokeClass = isSelected ? 'stroke-text-primary' : 'stroke-text-muted';

  return (
    <svg
      className={cn('h-4 w-4 fill-none', strokeClass)}
      viewBox="0 0 24 24"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      aria-hidden="true"
    >
      {kind === 'skill' ? (
        <path
          d="M13 10V3L4 14h7v7l9-11h-7z"
          fill="currentColor"
          stroke="none"
          className={isSelected ? 'text-text-primary' : 'text-text-muted'}
        />
      ) : kind === 'command' ? (
        <path d="M8 8 4 12l4 4M16 8l4 4-4 4M13 5l-2 14" />
      ) : (
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
      )}
    </svg>
  );
}
