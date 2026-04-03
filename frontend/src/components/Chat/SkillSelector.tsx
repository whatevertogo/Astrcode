/**
 * Skill 选择器弹出面板组件
 *
 * 当用户在输入框中输入 '/' 时触发，展示当前会话可用的技能列表。
 * 支持键盘 ↑↓ 导航、Enter 确认选中、Escape 关闭。
 * 样式适配 AstrCode 暖色/米色主题风格。
 *
 * 后端已完整提供：
 * - API: GET /api/sessions/{id}/composer/options?kinds=skill&q={query}
 * - 类型: ComposerOptionDto (protocol) → ComposerOption (frontend)
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ComposerOption } from '../../types';

interface SkillSelectorProps {
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

export default function SkillSelector({
  visible,
  options,
  loading,
  onSelect,
  onClose,
  query,
}: SkillSelectorProps) {
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
          setSelectedIndex((prev) => Math.min(prev + 1, filteredOptions.length - 1));
          break;
        case 'ArrowUp':
          e.preventDefault();
          e.stopPropagation();
          setSelectedIndex((prev) => Math.max(prev - 1, 0));
          break;
        case 'Enter':
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

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
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
      className="absolute bottom-full left-0 right-0 mb-2 max-h-64 overflow-y-auto rounded-xl border border-[var(--border)] bg-[var(--panel-bg)] shadow-[0_24px_42px_rgba(117,90,52,0.12)] py-1.5 z-50"
      ref={panelRef}
      onMouseDown={(e) => e.preventDefault()}
      role="listbox"
      aria-label="技能选择"
    >
      {loading ? (
        <div className="flex items-center justify-center py-4 text-xs text-[var(--text-muted)]">
          加载中...
        </div>
      ) : filteredOptions.length === 0 ? (
        <div className="px-3 py-2 text-xs text-[var(--text-faint)]">
          没有找到匹配「{query}」的技能
        </div>
      ) : (
        filteredOptions.map((option, index) => (
          <button
            key={option.id}
            type="button"
            role="option"
            aria-selected={index === selectedIndex}
            data-index={index}
            onMouseEnter={() => setSelectedIndex(index)}
            onClick={() => onSelect(option)}
            className={`w-full flex items-start gap-3 px-3 py-2.5 text-left transition-colors duration-100 ${
              index === selectedIndex
                ? 'bg-[var(--accent-soft)]/10 text-[var(--accent-strong)]'
                : 'text-[var(--text-primary)] hover:bg-[var(--surface-muted)]'
            }`}
          >
            <span className="flex-shrink-0 mt-1">
              <SkillIcon kind={option.kind} isSelected={index === selectedIndex} />
            </span>
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="font-medium text-[13px] leading-snug">{option.title}</span>
                {option.badges?.map((badge) => (
                  <span
                    key={badge}
                    className="inline-block px-1.5 py-0 text-[10px] rounded-full border border-[var(--border)] bg-[var(--surface)] text-[var(--text-muted)]"
                  >
                    {badge}
                  </span>
                ))}
              </div>
              <p className="mt-1 text-[12px] leading-relaxed text-[var(--text-muted)] line-clamp-2">
                {option.description}
              </p>
            </div>
          </button>
        ))
      )}
    </div>
  );
}

/**
 * Skill / Capability 图标，根据 kind 展示不同图标
 */
function SkillIcon({ kind, isSelected }: { kind: ComposerOption['kind']; isSelected: boolean }) {
  const color = isSelected
    ? 'stroke-[var(--accent-soft)]'
    : kind === 'skill'
      ? 'stroke-[var(--text-muted)]'
      : kind === 'capability'
        ? 'stroke-[var(--text-secondary)]'
        : 'stroke-[var(--text-faint)]';

  const fill = isSelected && kind === 'skill' ? 'fill-[var(--accent-soft)]/15' : 'fill-none';

  return (
    <svg
      className={`h-4 w-4 ${color} ${fill}`}
      viewBox="0 0 24 24"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      aria-hidden="true"
    >
      {kind === 'skill' ? (
        <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
      ) : (
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
      )}
    </svg>
  );
}
