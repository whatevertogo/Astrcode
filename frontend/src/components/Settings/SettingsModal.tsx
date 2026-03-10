import React, { useEffect, useMemo, useState } from 'react';
import type { ConfigView, ProfileView, TestResult } from '../../types';
import styles from './SettingsModal.module.css';

interface SettingsModalProps {
  onClose: () => void;
  getConfig: () => Promise<ConfigView>;
  saveActiveSelection: (activeProfile: string, activeModel: string) => Promise<void>;
  testConnection: (profileName: string, model: string) => Promise<TestResult>;
  openConfigInEditor: () => Promise<void>;
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

  const profiles = configView?.profiles ?? [];
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
      await openConfigInEditor();
    } catch (error) {
      setErrorMessage(String(error));
    }
  };

  return (
    <div
      className={styles.overlay}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className={styles.modal}>
        {warning && <div className={styles.warningBanner}>{warning}</div>}

        <div className={styles.header}>
          <div className={styles.title}>设置</div>
          <button
            type="button"
            className={styles.closeButton}
            onClick={onClose}
            aria-label="关闭设置"
          >
            ×
          </button>
        </div>

        {loading ? (
          <div className={styles.loadingState}>
            <span className={styles.spinner} />
            正在读取配置...
          </div>
        ) : (
          <>
            <div className={styles.section}>
              <label className={styles.label}>配置文件</label>
              <div className={styles.pathRow}>
                <div className={styles.pathValue}>{configView?.configPath ?? ''}</div>
                <button
                  type="button"
                  className={styles.secondaryButton}
                  onClick={() => void handleOpenConfig()}
                >
                  在编辑器中打开
                </button>
              </div>
            </div>

            <div className={styles.section}>
              <label className={styles.label}>Profile</label>
              <select
                className={styles.select}
                value={selectedProfile}
                onChange={handleProfileChange}
              >
                {profiles.map((profile) => (
                  <option key={profile.name} value={profile.name}>
                    {profile.name}
                  </option>
                ))}
              </select>
            </div>

            <div className={styles.section}>
              <label className={styles.label}>Model</label>
              <select
                className={styles.select}
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

            <div className={styles.section}>
              <div className={styles.infoRow}>
                <span className={styles.infoLabel}>Base URL</span>
                <span className={styles.infoValue}>{currentProfile?.baseUrl ?? '-'}</span>
              </div>
              <div className={styles.infoRow}>
                <span className={styles.infoLabel}>API Key</span>
                <span className={styles.infoValue}>
                  {currentProfile?.apiKeyPreview ?? '未配置'}
                </span>
              </div>
            </div>

            <div className={styles.actions}>
              <button
                type="button"
                className={styles.secondaryButton}
                onClick={() => void handleTestConnection()}
                disabled={testing || saving || !selectedProfile || !selectedModel}
              >
                测试连接
              </button>
              <button
                type="button"
                className={styles.primaryButton}
                onClick={() => void handleSave()}
                disabled={saving || testing || !selectedProfile || !selectedModel}
              >
                {saving ? '保存中...' : '保存'}
              </button>
            </div>

            <div className={styles.resultArea}>
              {testing && (
                <div className={styles.loadingState}>
                  <span className={styles.spinner} />
                  正在测试连接...
                </div>
              )}
              {!testing && testResult?.success && (
                <div className={styles.successMessage}>
                  {'\u2705'} 连接成功 · {testResult.provider} · {testResult.model}
                </div>
              )}
              {!testing && testResult && !testResult.success && (
                <div className={styles.errorMessage}>
                  {'\u274c'} {testResult.error ?? '连接失败'}
                </div>
              )}
              {errorMessage && <div className={styles.errorMessage}>{errorMessage}</div>}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
