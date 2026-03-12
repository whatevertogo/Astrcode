import React, { useEffect, useMemo, useRef, useState } from 'react';
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
  const [currentModel, setCurrentModel] = useState<CurrentModelInfo | null>(null);
  const [options, setOptions] = useState<ModelOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
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

  // Flatten grouped options with indices for stable key lookup
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

  // Find the index of the currently selected model
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

  const handleChange = async (event: React.ChangeEvent<HTMLSelectElement>) => {
    const index = parseInt(event.target.value, 10);
    if (isNaN(index) || index < 0 || index >= flattenedOptions.length) {
      return;
    }

    const { option } = flattenedOptions[index];

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
    <div className={styles.wrapper}>
      <select
        className={styles.select}
        value={selectedValue}
        onChange={(event) => void handleChange(event)}
        disabled={loading || saving || options.length <= 1}
        aria-label="选择模型"
      >
        {currentModel === null && (
          <option value="">{loading ? '读取模型中...' : '未选择模型'}</option>
        )}
        {groupedOptions.map(([profileName, profileOptions]) => (
          <optgroup key={profileName} label={profileName}>
            {profileOptions.map(({ option, index }) => (
              <option key={optionKey(index)} value={optionKey(index)}>
                {`${option.model} · ${option.providerKind}`}
              </option>
            ))}
          </optgroup>
        ))}
      </select>
      {error && <span className={styles.error}>{error}</span>}
    </div>
  );
}
