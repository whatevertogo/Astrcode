import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { SubRunFinishMessage, SubRunStartMessage } from '../../types';
import type { ThreadItem } from '../../lib/subRunView';
import styles from './SubRunBlock.module.css';

interface SubRunBlockProps {
  subRunId: string;
  sessionId: string | null;
  title: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  threadItems: ThreadItem[];
  streamFingerprint: string;
  hasDescriptorLineage: boolean;
  renderThreadItems: (
    items: ThreadItem[],
    options?: {
      nested?: boolean;
    }
  ) => React.ReactNode[];
  onCancelSubRun: (sessionId: string, subRunId: string) => void | Promise<void>;
  onFocusSubRun?: (subRunId: string) => void;
  onOpenChildSession?: (childSessionId: string) => void | Promise<void>;
  displayMode?: 'thread' | 'directory';
}

type SubRunStatus = 'running' | 'completed' | 'aborted' | 'token_exceeded' | 'failed';

function toSubRunStatus(finishMessage?: SubRunFinishMessage): SubRunStatus {
  return finishMessage?.result.status ?? 'running';
}

function getStatusLabel(status: SubRunStatus): string {
  switch (status) {
    case 'completed':
      return 'completed';
    case 'aborted':
      return 'aborted';
    case 'token_exceeded':
      return 'token exceeded';
    case 'failed':
      return 'failed';
    case 'running':
    default:
      return 'running';
  }
}

function getStorageModeLabel(startMessage?: SubRunStartMessage): string {
  if (!startMessage) {
    return 'shared session';
  }
  return startMessage.storageMode === 'independentSession'
    ? 'independent session'
    : 'shared session';
}

function getStatusClassName(status: SubRunStatus): string {
  switch (status) {
    case 'completed':
      return styles.statusCompleted;
    case 'aborted':
      return styles.statusAborted;
    case 'token_exceeded':
      return styles.statusTokenExceeded;
    case 'failed':
      return styles.statusFailed;
    case 'running':
    default:
      return styles.statusRunning;
  }
}

function isVisibleActivityItem(item: ThreadItem): boolean {
  if (item.kind === 'subRun') {
    return true;
  }

  return item.message.kind === 'assistant' || item.message.kind === 'toolCall';
}

function SubRunBlock({
  subRunId,
  sessionId,
  title,
  startMessage,
  finishMessage,
  threadItems,
  streamFingerprint,
  hasDescriptorLineage: _hasDescriptorLineage,
  renderThreadItems,
  onCancelSubRun,
  onFocusSubRun,
  onOpenChildSession,
  displayMode = 'thread',
}: SubRunBlockProps) {
  const [userInteracted, setUserInteracted] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const detailsRef = useRef<HTMLDetailsElement>(null);
  const streamRef = useRef<HTMLDivElement>(null);
  const shouldStickToBottomRef = useRef(true);
  const previousFingerprintRef = useRef('');

  const status = toSubRunStatus(finishMessage);
  const statusLabel = getStatusLabel(status);
  const metrics =
    finishMessage !== undefined
      ? `${finishMessage.stepCount} steps · ${finishMessage.estimatedTokens} tokens`
      : getStorageModeLabel(startMessage);
  const resultHandoff = finishMessage?.result.handoff;
  const resultFailure = finishMessage?.result.failure;
  const isBackgroundRunning = status === 'running';
  const childSessionId = startMessage?.childSessionId ?? finishMessage?.childSessionId;
  const navigationLabel =
    childSessionId !== undefined
      ? '打开独立会话'
      : displayMode === 'directory'
        ? '进入子执行'
        : '查看子执行';
  const activityItems = useMemo(() => threadItems.filter(isVisibleActivityItem), [threadItems]);
  const activitySummary =
    resultFailure?.displayMessage ||
    resultHandoff?.summary.trim() ||
    (isBackgroundRunning ? '后台运行中，展开后会继续实时刷新。' : '展开查看子执行的思考和工具流。');
  const shouldAutoOpen = !userInteracted && isBackgroundRunning;

  const updateStreamStickiness = useCallback(() => {
    const container = streamRef.current;
    if (!container) {
      shouldStickToBottomRef.current = true;
      return;
    }
    const distanceFromBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    shouldStickToBottomRef.current = distanceFromBottom <= 48;
  }, []);

  useEffect(() => {
    updateStreamStickiness();
  }, [updateStreamStickiness]);

  useEffect(() => {
    const container = streamRef.current;
    if (!container) {
      return;
    }
    const onWheel = (e: WheelEvent) => {
      const atTop = container.scrollTop <= 0 && e.deltaY < 0;
      const atBottom =
        container.scrollTop + container.clientHeight >= container.scrollHeight - 1 && e.deltaY > 0;
      if (atTop || atBottom) {
        e.preventDefault();
      }
    };
    container.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      container.removeEventListener('wheel', onWheel);
    };
  }, []);

  useEffect(() => {
    if (previousFingerprintRef.current === streamFingerprint) {
      return;
    }
    previousFingerprintRef.current = streamFingerprint;
    if (!shouldStickToBottomRef.current) {
      return;
    }
    const container = streamRef.current;
    if (!container) {
      return;
    }
    const rafId = window.requestAnimationFrame(() => {
      container.scrollTop = container.scrollHeight;
      updateStreamStickiness();
    });
    return () => window.cancelAnimationFrame(rafId);
  }, [streamFingerprint, updateStreamStickiness]);

  useEffect(() => {
    if (!shouldAutoOpen) {
      return;
    }
    const details = detailsRef.current;
    if (!details || details.open) {
      return;
    }
    details.open = true;
  }, [shouldAutoOpen]);

  const handleCancel = useCallback(async () => {
    if (!sessionId || cancelling) {
      return;
    }
    setCancelling(true);
    setCancelError(null);
    try {
      await onCancelSubRun(sessionId, subRunId);
    } catch (error) {
      setCancelError(error instanceof Error ? error.message : String(error));
    } finally {
      setCancelling(false);
    }
  }, [cancelling, onCancelSubRun, sessionId, subRunId]);

  const handleOpenView = useCallback(async () => {
    if (childSessionId) {
      await onOpenChildSession?.(childSessionId);
      return;
    }
    onFocusSubRun?.(subRunId);
  }, [childSessionId, onFocusSubRun, onOpenChildSession, subRunId]);

  const renderToolbar = () => (
    <div className={styles.toolbar}>
      <div className={styles.toolbarText}>{activitySummary}</div>
      <div className={styles.toolbarActions}>
        {(onFocusSubRun || (childSessionId && onOpenChildSession)) && (
          <button type="button" className={styles.openButton} onClick={() => void handleOpenView()}>
            {navigationLabel}
          </button>
        )}
        {sessionId && isBackgroundRunning && (
          <button
            type="button"
            className={styles.cancelButton}
            onClick={() => void handleCancel()}
            disabled={cancelling}
          >
            {cancelling ? '取消中...' : '取消子会话'}
          </button>
        )}
      </div>
    </div>
  );

  const renderActivity = () => (
    <details
      className={styles.activitySection}
      open={isBackgroundRunning || activityItems.length > 0}
    >
      <summary className={styles.activitySummary}>
        <span>思考与工具</span>
        <span className={styles.activityMeta}>
          {activityItems.length > 0 ? `${activityItems.length} 条活动` : '等待输出'}
        </span>
        <span className={styles.summaryChevron}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="9 18 15 12 9 6"></polyline>
          </svg>
        </span>
      </summary>
      <div ref={streamRef} className={styles.activityBody} onScroll={updateStreamStickiness}>
        {activityItems.length === 0 ? (
          <div className={styles.activityEmpty}>
            {isBackgroundRunning
              ? '等待子 Agent 输出思考或工具调用...'
              : '该子执行没有产生可展示的思考或工具调用。'}
          </div>
        ) : (
          renderThreadItems(activityItems, { nested: true })
        )}
      </div>
    </details>
  );

  const renderFinalReply = () => {
    // 成功交付时展示最终回复摘要
    if (!resultHandoff || status !== 'completed') {
      return null;
    }
    return (
      <details className={styles.activitySection} open>
        <summary className={styles.activitySummary}>
          <span>最终回复</span>
          <span className={styles.summaryChevron}>
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="9 18 15 12 9 6"></polyline>
            </svg>
          </span>
        </summary>
        <div className={styles.activityBody}>
          <div className={styles.toolbarText}>{resultHandoff.summary}</div>
          {resultHandoff.findings.length > 0 && (
            <ul className={styles.findingsList}>
              {resultHandoff.findings.map((finding, index) => (
                <li key={index}>{finding}</li>
              ))}
            </ul>
          )}
        </div>
      </details>
    );
  };

  return (
    <details
      ref={detailsRef}
      className={styles.wrapper}
      onToggle={(event) => {
        if (event.target === event.currentTarget && event.nativeEvent.isTrusted) {
          setUserInteracted(true);
        }
      }}
    >
      <summary className={styles.summary} title={`${title} · ${statusLabel} · ${metrics}`}>
        <span className={styles.summaryText}>子 Agent {title}</span>
        <span className={`${styles.statusPill} ${getStatusClassName(status)}`}>{statusLabel}</span>
        <span className={styles.summaryMeta}>{metrics}</span>
        <span className={styles.summaryChevron}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="9 18 15 12 9 6"></polyline>
          </svg>
        </span>
      </summary>

      <div className={styles.body}>
        {displayMode === 'directory' ? (
          <div className={styles.directoryCard}>
            <div className={styles.toolbarText}>{activitySummary}</div>
            {(onFocusSubRun || (childSessionId && onOpenChildSession)) && (
              <button
                type="button"
                className={styles.openButton}
                onClick={() => void handleOpenView()}
              >
                {navigationLabel}
              </button>
            )}
          </div>
        ) : (
          <>
            {renderToolbar()}
            {cancelError && <div className={styles.resultError}>{cancelError}</div>}
            {resultFailure && (
              <div className={styles.failureCard}>
                <div className={styles.failureTitle}>执行失败</div>
                <div className={styles.failureMessage}>{resultFailure.displayMessage}</div>
                {resultFailure.technicalMessage && (
                  <details className={styles.activitySection}>
                    <summary className={styles.activitySummary}>
                      <span>技术详情</span>
                      <span className={styles.summaryChevron}>
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <polyline points="9 18 15 12 9 6"></polyline>
                        </svg>
                      </span>
                    </summary>
                    <div className={styles.activityBody}>
                      <div className={styles.resultError}>{resultFailure.technicalMessage}</div>
                    </div>
                  </details>
                )}
              </div>
            )}
            {renderFinalReply()}
            {renderActivity()}
          </>
        )}
      </div>
    </details>
  );
}

export default memo(SubRunBlock);
