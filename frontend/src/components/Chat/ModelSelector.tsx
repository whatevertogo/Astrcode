import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CurrentModelInfo, ModelOption } from '../../types';
import styles from './ModelSelector.module.css';

interface ModelSelectorProps {
  refreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

/**
 * Creates a stable key for an option using its index in the flattened options array.
 * This avoids issues with special characters in profile/model names.
 */
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
  const [currentModel, setCurrentModel] = useState<CurrentModelInfo | null>(null);
  const [options, setOptions] = useState<ModelOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const errorTimerRef = useRef<number | null>(null);

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

  const groupedOptions = useMemo(() => {
    const groups = new Map<string, Array<{ option: ModelOption; index: number }>>();
    for (const { option, index, profileName } of flattenedOptions) {
      const group = groups.get(profileName) ?? [];
      group.push({ option, index });
      groups.set(profileName, group);
    }
    return Array.from(groups.entries());
  }, [flattenedOptions]);

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
    <div ref={wrapperRef} className={`${styles.wrapper} ${open ? styles.wrapperOpen : ''}`}>
      <button
        type="button"
        className={styles.trigger}
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
        <span className={styles.leadingIcon} aria-hidden="true">
          <svg viewBox="0 0 20 20">
            <path
              d="M8.75 2.5h2.5l.39 1.94c.38.11.74.26 1.08.45l1.84-.73 1.26 2.18-1.45 1.24c.03.19.05.37.05.56 0 .19-.02.37-.05.56l1.45 1.24-1.26 2.18-1.84-.73c-.34.19-.7.34-1.08.45l-.39 1.94h-2.5l-.39-1.94a4.96 4.96 0 0 1-1.08-.45l-1.84.73-1.26-2.18 1.45-1.24a3.7 3.7 0 0 1-.05-.56c0-.19.02-.37.05-.56L3.31 6.34l1.26-2.18 1.84.73c.34-.19.7-.34 1.08-.45l.39-1.94ZM10 7.3A2.7 2.7 0 1 0 10 12.7 2.7 2.7 0 0 0 10 7.3Z"
              fill="currentColor"
            />
          </svg>
        </span>
        <span className={styles.triggerContent}>
          <span className={styles.triggerLabel}>
            {selectedOption?.model ?? (loading ? '读取模型中...' : '未选择模型')}
          </span>
          <span className={styles.triggerMeta}>
            {selectedOption
              ? `${selectedOption.profileName} · ${selectedOption.providerKind}`
              : loading
                ? '同步当前配置...'
                : '未检测到可用模型'}
          </span>
        </span>
        <span className={styles.triggerBadge}>
          {saving ? '切换中' : (selectedOption?.providerKind ?? 'model')}
        </span>
        <span className={styles.chevron} aria-hidden="true">
          <svg viewBox="0 0 12 12">
            <path
              d="M2.5 4.5 6 8l3.5-3.5"
              fill="none"
              stroke="currentColor"
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth="1.4"
            />
          </svg>
        </span>
      </button>

      {open && groupedOptions.length > 0 && (
        <div id={listboxId} className={styles.dropdown} role="listbox" aria-label="模型列表">
          {groupedOptions.map(([profileName, profileOptions]) => (
            <div key={profileName} className={styles.group}>
              <div className={styles.groupLabel}>{profileName}</div>
              <div className={styles.groupOptions}>
                {profileOptions.map(({ option, index }) => {
                  const isSelected = optionKey(index) === selectedValue;
                  return (
                    <button
                      key={optionKey(index)}
                      type="button"
                      className={`${styles.optionButton} ${isSelected ? styles.optionButtonSelected : ''}`}
                      onClick={() => void handleSelect(index)}
                      role="option"
                      aria-selected={isSelected}
                    >
                      <span className={styles.optionMain}>
                        <span className={styles.optionTitle}>{option.model}</span>
                        <span className={styles.optionMeta}>{option.providerKind}</span>
                      </span>
                      <span className={styles.optionCheck} aria-hidden="true">
                        {isSelected ? '当前' : '切换'}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      )}

      {error && <span className={styles.error}>{error}</span>}
    </div>
  );
}
