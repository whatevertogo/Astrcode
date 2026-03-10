import React, { useEffect, useMemo, useRef, useState } from 'react';
import type { CurrentModelInfo, ModelOption } from '../../types';
import styles from './ModelSelector.module.css';

interface ModelSelectorProps {
  refreshKey: number;
  getCurrentModel: () => Promise<CurrentModelInfo>;
  listAvailableModels: () => Promise<ModelOption[]>;
  setModel: (profileName: string, model: string) => Promise<void>;
}

function optionValue(profileName: string, model: string): string {
  return `${profileName}::${model}`;
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
      if (errorTimerRef.current != null) {
        window.clearTimeout(errorTimerRef.current);
      }
    };
  }, []);

  const showError = (message: string) => {
    setError(message);
    if (errorTimerRef.current != null) {
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

  const groupedOptions = useMemo(() => {
    const groups = new Map<string, ModelOption[]>();
    for (const option of options) {
      const group = groups.get(option.profileName) ?? [];
      group.push(option);
      groups.set(option.profileName, group);
    }
    return Array.from(groups.entries());
  }, [options]);

  const selectedValue = currentModel
    ? optionValue(currentModel.profileName, currentModel.model)
    : '';

  const handleChange = async (event: React.ChangeEvent<HTMLSelectElement>) => {
    const [profileName, model] = event.target.value.split('::');
    if (!profileName || !model) {
      return;
    }

    setSaving(true);
    try {
      await setModel(profileName, model);
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
        {currentModel == null && (
          <option value="">
            {loading ? '读取模型中...' : '未选择模型'}
          </option>
        )}
        {groupedOptions.map(([profileName, profileOptions]) => (
          <optgroup key={profileName} label={profileName}>
            {profileOptions.map((option) => (
              <option
                key={optionValue(option.profileName, option.model)}
                value={optionValue(option.profileName, option.model)}
              >
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
