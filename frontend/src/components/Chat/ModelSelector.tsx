import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CurrentModelInfo, ModelOption } from '../../types';
import { cn } from '../../lib/utils';

interface ModelSelectorProps {
  refreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

function optionKey(index: number): string {
  return String(index);
}

export default function ModelSelector({
  refreshKey,
  getCurrentModel,
  listAvailableModels,
  setModel,
}: ModelSelectorProps) {
  const listboxId = useId();
  const wrapperRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [currentModel, setCurrentModel] = useState<CurrentModelInfo | null>(null);
  const [options, setOptions] = useState<ModelOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const errorTimerRef = useRef<number | null>(null);
  const [hoverIndex, setHoverIndex] = useState<number>(-1);

  useEffect(() => {
    return () => {
      if (errorTimerRef.current !== null) {
        window.clearTimeout(errorTimerRef.current);
      }
    };
  }, []);

  const showError = (message: string) => {
    setError(message);
    if (errorTimerRef.current !== null) {
      window.clearTimeout(errorTimerRef.current);
    }
    errorTimerRef.current = window.setTimeout(() => {
      setError(null);
      errorTimerRef.current = null;
    }, 2500);
  };

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      try {
        const [nextOptions, nextCurrentModel] = await Promise.all([
          listAvailableModels(),
          getCurrentModel(),
        ]);
        if (cancelled) {
          return;
        }
        setOptions(nextOptions);
        setCurrentModel(nextCurrentModel);
        setError(null);
      } catch (loadError) {
        if (!cancelled) {
          showError(String(loadError));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [getCurrentModel, listAvailableModels, refreshKey]);

  useEffect(() => {
    if (!open) {
      setSearchQuery('');
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      if (!(event.target instanceof Node)) {
        return;
      }
      if (!wrapperRef.current?.contains(event.target)) {
        setOpen(false);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);

    // 聚焦搜索框
    requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  const flattenedOptions = useMemo(() => {
    const result: Array<{ option: ModelOption; index: number; profileName: string }> = [];
    for (const option of options) {
      result.push({
        option,
        index: result.length,
        profileName: option.profileName,
      });
    }
    return result;
  }, [options]);

  const filteredOptions = useMemo(() => {
    if (!searchQuery) return flattenedOptions;
    const q = searchQuery.toLowerCase();
    return flattenedOptions.filter(
      (item) =>
        item.option.model.toLowerCase().includes(q) ||
        item.profileName.toLowerCase().includes(q) ||
        item.option.providerKind.toLowerCase().includes(q)
    );
  }, [flattenedOptions, searchQuery]);

  const groupedOptions = useMemo(() => {
    const groups = new Map<string, Array<{ option: ModelOption; index: number }>>();
    for (const { option, index, profileName } of filteredOptions) {
      const group = groups.get(profileName) ?? [];
      group.push({ option, index });
      groups.set(profileName, group);
    }
    return Array.from(groups.entries());
  }, [filteredOptions]);

  const selectedIndex = useMemo(() => {
    if (currentModel === null) {
      return -1;
    }
    return flattenedOptions.findIndex(
      (item) =>
        item.option.profileName === currentModel.profileName &&
        item.option.model === currentModel.model
    );
  }, [currentModel, flattenedOptions]);

  const selectedValue = selectedIndex >= 0 ? optionKey(selectedIndex) : '';
  const selectedOption =
    selectedIndex >= 0 ? (flattenedOptions[selectedIndex]?.option ?? null) : null;
  const triggerDisabled = loading || saving || options.length === 0;

  const handleSelect = async (index: number) => {
    if (index < 0 || index >= flattenedOptions.length) {
      return;
    }

    const { option } = flattenedOptions[index];
    setOpen(false);

    setSaving(true);
    try {
      await setModel(option.profileName, option.model);
      const refreshed = await getCurrentModel();
      setCurrentModel(refreshed);
      setError(null);
    } catch (changeError) {
      showError(String(changeError));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div ref={wrapperRef} className="relative">
      <button
        type="button"
        className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg hover:bg-[rgba(0,0,0,0.05)] text-[13px] text-text-secondary transition-all duration-150 ease-out focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-soft/30 disabled:opacity-60 disabled:cursor-not-allowed"
        onClick={() => {
          if (!triggerDisabled) {
            setOpen((value) => !value);
          }
        }}
        disabled={triggerDisabled}
        aria-label="选择模型"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
      >
        <span className="truncate max-w-[200px] text-text-primary font-medium">
          {selectedOption?.model ?? (loading ? '加载中...' : '未选择模型')}
        </span>
        <svg
          className={`w-4 h-4 transition-transform duration-200 ${open ? 'rotate-180' : ''}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="6 9 12 15 18 9"></polyline>
        </svg>
      </button>

      {open && (
        <div
          id={listboxId}
          className="absolute bottom-[calc(100%+8px)] left-0 w-[240px] bg-surface border border-border rounded-2xl shadow-[0_12px_32px_rgba(117,90,52,0.1)] z-[9999] flex flex-col origin-bottom-left animate-in fade-in zoom-in-95 duration-100"
        >
          <div className="p-1.5 border-b border-border">
            <div className="relative flex items-center">
              <svg
                className="absolute left-2.5 w-3.5 h-3.5 text-text-muted"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <circle cx="11" cy="11" r="8"></circle>
                <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
              </svg>
              <input
                ref={searchInputRef}
                type="text"
                placeholder="搜索模型..."
                className="w-full bg-transparent border-none py-1.5 pl-7 pr-2.5 text-[13px] text-text-primary focus:outline-none placeholder:text-text-muted"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>
          </div>

          <div
            className="overflow-y-auto p-1.5 flex-1 max-h-[240px] [&::-webkit-scrollbar]:hidden"
            style={{ scrollbarWidth: 'none', msOverflowStyle: 'none' }}
          >
            {groupedOptions.length === 0 ? (
              <div className="px-3 py-6 text-center text-[12px] text-text-muted">无结果</div>
            ) : (
              groupedOptions.map(([profileName, profileOptions], groupIdx) => (
                <div key={profileName} className={groupIdx > 0 ? 'mt-1.5' : ''}>
                  <div className="px-4 py-1.5 text-[11px] font-semibold text-text-muted tracking-wider uppercase select-none">
                    {profileName}
                  </div>
                  <div className="flex flex-col gap-0.5 px-1.5">
                    {profileOptions.map(({ option, index }) => {
                      const isSelected = optionKey(index) === selectedValue;
                      return (
                        <button
                          key={optionKey(index)}
                          type="button"
                          onMouseEnter={() => setHoverIndex(index)}
                          onMouseLeave={() => setHoverIndex(-1)}
                          className={cn(
                            'w-full flex items-center justify-between px-3 h-[34px] text-left rounded-lg transition-colors duration-100 text-text-primary',
                            index === hoverIndex && 'bg-[rgba(0,0,0,0.05)]'
                          )}
                          onClick={() => void handleSelect(index)}
                          role="option"
                          aria-selected={isSelected}
                        >
                          <span className="text-[13px] font-medium truncate leading-normal">
                            {option.model}
                          </span>
                          {isSelected && (
                            <svg
                              className="w-4 h-4 text-text-primary flex-shrink-0"
                              viewBox="0 0 24 24"
                              fill="none"
                              stroke="currentColor"
                              strokeWidth="2.5"
                              strokeLinecap="round"
                              strokeLinejoin="round"
                            >
                              <polyline points="20 6 9 17 4 12"></polyline>
                            </svg>
                          )}
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))
            )}
          </div>
          {error && (
            <div className="mx-1.5 mb-1.5 p-2 bg-danger-soft text-danger rounded-lg text-[12px] text-center border border-danger-soft">
              {error}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
