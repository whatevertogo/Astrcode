import React, { useEffect, useMemo, useState } from 'react';
import type { ConfigView, ProfileView, TestResult } from '../../types';
import { btnPrimary, btnSecondary, dialogSurface, fieldInput, overlay } from '../../lib/styles';
import { cn } from '../../lib/utils';

interface SettingsModalProps {
  onClose: () => void;
  getConfig: () => Promise<ConfigView>;
  saveActiveSelection: (activeProfile: string, activeModel: string) => Promise<void>;
  testConnection: (profileName: string, model: string) => Promise<TestResult>;
  openConfigInEditor: (path?: string) => Promise<void>;
}

function pickNextModel(profile: ProfileView | undefined, currentModel: string): string {
  if (!profile || profile.models.length === 0) {
    return '';
  }
  if (profile.models.includes(currentModel)) {
    return currentModel;
  }
  return profile.models[0];
}

export default function SettingsModal({
  onClose,
  getConfig,
  saveActiveSelection,
  testConnection,
  openConfigInEditor,
}: SettingsModalProps) {
  const [configView, setConfigView] = useState<ConfigView | null>(null);
  const [selectedProfile, setSelectedProfile] = useState('');
  const [selectedModel, setSelectedModel] = useState('');
  const [testResult, setTestResult] = useState<TestResult | null>(null);
  const [warning, setWarning] = useState<string | undefined>();
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      setErrorMessage(null);
      try {
        const nextConfig = await getConfig();
        if (cancelled) {
          return;
        }
        setConfigView(nextConfig);
        setSelectedProfile(nextConfig.activeProfile);
        setSelectedModel(nextConfig.activeModel);
        setWarning(nextConfig.warning);
        setTestResult(null);
      } catch (error) {
        if (!cancelled) {
          setErrorMessage(String(error));
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
  }, [getConfig]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const profiles = useMemo(() => configView?.profiles ?? [], [configView]);
  const currentProfile = useMemo(
    () => profiles.find((profile) => profile.name === selectedProfile) ?? profiles[0],
    [profiles, selectedProfile]
  );

  useEffect(() => {
    const nextModel = pickNextModel(currentProfile, selectedModel);
    if (nextModel !== selectedModel) {
      setSelectedModel(nextModel);
    }
  }, [currentProfile, selectedModel]);

  const handleProfileChange = (event: React.ChangeEvent<HTMLSelectElement>) => {
    const profileName = event.target.value;
    const profile = profiles.find((item) => item.name === profileName);
    setSelectedProfile(profileName);
    setSelectedModel(pickNextModel(profile, selectedModel));
    setTestResult(null);
    setErrorMessage(null);
  };

  const handleModelChange = (event: React.ChangeEvent<HTMLSelectElement>) => {
    setSelectedModel(event.target.value);
    setTestResult(null);
    setErrorMessage(null);
  };

  const handleTestConnection = async () => {
    if (!selectedProfile || !selectedModel) {
      return;
    }
    setTesting(true);
    setErrorMessage(null);
    setTestResult(null);
    try {
      const result = await testConnection(selectedProfile, selectedModel);
      setTestResult(result);
    } catch (error) {
      setErrorMessage(String(error));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    if (!selectedProfile || !selectedModel) {
      return;
    }
    setSaving(true);
    setErrorMessage(null);
    try {
      await saveActiveSelection(selectedProfile, selectedModel);
      setWarning(undefined);
      setTestResult(null);
      onClose();
    } catch (error) {
      setErrorMessage(String(error));
    } finally {
      setSaving(false);
    }
  };

  const handleOpenConfig = async () => {
    setErrorMessage(null);
    try {
      await openConfigInEditor(configView?.configPath);
    } catch (error) {
      setErrorMessage(String(error));
    }
  };

  return (
    <div
      className={overlay}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className={cn(
          dialogSurface,
          'w-[520px] max-w-[min(520px,100%)] sm:max-w-[calc(100vw-20px)] sm:p-4'
        )}
      >
        {warning && (
          <div className="mb-3.5 rounded-[14px] border border-warning-border bg-warning-soft px-3.5 py-3 text-xs leading-relaxed text-warning">
            {warning}
          </div>
        )}

        <div className="flex items-center justify-between gap-3 mb-[18px]">
          <div className="text-xl font-bold text-text-primary">设置</div>
          <button
            type="button"
            className="w-8 h-8 rounded-[10px] bg-surface-soft text-text-secondary border border-border text-lg transition-[background-color,color,border-color] duration-150 ease-out hover:bg-white hover:text-text-primary hover:border-border-strong"
            onClick={onClose}
            aria-label="关闭设置"
          >
            ×
          </button>
        </div>

        {loading ? (
          <div className="flex items-center gap-2 text-xs text-text-secondary leading-relaxed">
            <span className="h-[14px] w-[14px] animate-spin rounded-full border-2 border-border border-t-text-secondary" />
            正在读取配置...
          </div>
        ) : (
          <>
            <div className="mb-4">
              <label className="block mb-2 text-text-secondary text-[13px] font-semibold">
                配置文件
              </label>
              <div className="flex gap-2.5 items-center sm:flex-col sm:items-stretch">
                <div className="flex-1 min-w-0 py-[11px] px-3 rounded-xl border border-border bg-surface text-text-primary text-xs overflow-hidden text-ellipsis whitespace-nowrap">
                  {configView?.configPath ?? ''}
                </div>
                <button
                  type="button"
                  className={btnSecondary}
                  onClick={() => void handleOpenConfig()}
                >
                  在编辑器中打开
                </button>
              </div>
            </div>

            <div className="mb-4">
              <label className="block mb-2 text-text-secondary text-[13px] font-semibold">
                Profile
              </label>
              <select className={fieldInput} value={selectedProfile} onChange={handleProfileChange}>
                {profiles.map((profile) => (
                  <option key={profile.name} value={profile.name}>
                    {profile.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="mb-4">
              <label className="block mb-2 text-text-secondary text-[13px] font-semibold">
                Model
              </label>
              <select
                className={cn(
                  fieldInput,
                  (!currentProfile || currentProfile.models.length === 0) && 'opacity-50'
                )}
                value={selectedModel}
                onChange={handleModelChange}
                disabled={!currentProfile || currentProfile.models.length === 0}
              >
                {(currentProfile?.models ?? []).map((model) => (
                  <option key={model} value={model}>
                    {model}
                  </option>
                ))}
              </select>
            </div>

            <div className="mb-4">
              <div className="flex justify-between gap-4 py-2.5 border-b border-border last:border-b-0">
                <span className="text-text-secondary text-xs">Base URL</span>
                <span className="text-text-primary text-xs text-right break-all">
                  {currentProfile?.baseUrl ?? '-'}
                </span>
              </div>
              <div className="flex justify-between gap-4 py-2.5">
                <span className="text-text-secondary text-xs">API Key</span>
                <span className="text-text-primary text-xs text-right break-all">
                  {currentProfile?.apiKeyPreview ?? '未配置'}
                </span>
              </div>
            </div>

            <div className="flex justify-end gap-2.5 mt-[22px] sm:flex-col">
              <button
                type="button"
                className={btnSecondary}
                onClick={() => void handleTestConnection()}
                disabled={testing || saving || !selectedProfile || !selectedModel}
              >
                测试连接
              </button>
              <button
                type="button"
                className={btnPrimary}
                onClick={() => void handleSave()}
                disabled={saving || testing || !selectedProfile || !selectedModel}
              >
                {saving ? '保存中...' : '保存'}
              </button>
            </div>

            <div className="min-h-7 mt-3.5">
              {testing && (
                <div className="flex items-center gap-2 text-xs text-text-secondary leading-relaxed">
                  <span className="h-[14px] w-[14px] animate-spin rounded-full border-2 border-border border-t-text-secondary" />
                  正在测试连接...
                </div>
              )}
              {!testing && testResult?.success && (
                <div className="flex items-center gap-2 text-xs text-success leading-relaxed">
                  {'\u2705'} 连接成功 · {testResult.provider} · {testResult.model}
                </div>
              )}
              {!testing && testResult && !testResult.success && (
                <div className="flex items-center gap-2 text-xs text-danger leading-relaxed">
                  {'\u274c'} {testResult.error ?? '连接失败'}
                </div>
              )}
              {errorMessage && (
                <div className="flex items-center gap-2 text-xs text-danger leading-relaxed">
                  {errorMessage}
                </div>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
