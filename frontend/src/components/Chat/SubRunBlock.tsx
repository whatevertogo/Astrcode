import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Message, SubRunFinishMessage, SubRunStartMessage } from '../../types';
import ToolJsonView from './ToolJsonView';
import styles from './SubRunBlock.module.css';

interface SubRunBlockProps {
  subRunId: string;
  sessionId: string | null;
  title: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  bodyMessages: Message[];
  renderMessageRow: (message: Message, previousMessage: Message | null) => React.ReactNode;
  onCancelSubRun: (sessionId: string, subRunId: string) => void | Promise<void>;
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

function buildMessageFingerprint(message: Message): string {
  if (message.kind === 'assistant') {
    return `${message.id}:assistant:${message.text.length}:${message.reasoningText?.length ?? 0}:${
      message.streaming ? 1 : 0
    }`;
  }
  if (message.kind === 'toolCall') {
    return `${message.id}:tool:${message.status}:${message.output?.length ?? 0}:${
      message.error?.length ?? 0
    }`;
  }
  if (message.kind === 'compact') {
    return `${message.id}:compact:${message.summary.length}`;
  }
  if (message.kind === 'user') {
    return `${message.id}:user:${message.text.length}`;
  }
  return `${message.id}:${message.kind}`;
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

function SubRunBlock({
  subRunId,
  sessionId,
  title,
  startMessage,
  finishMessage,
  bodyMessages,
  renderMessageRow,
  onCancelSubRun,
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
  const resultSummary = resultHandoff?.summary.trim() || '子会话未返回摘要。';
  const isBackgroundRunning = status === 'running';
  const visibleFindings = useMemo(
    () => (resultHandoff?.findings ?? []).filter((finding) => finding.trim().length > 0),
    [resultHandoff?.findings]
  );

  // 这里显式裁剪 undefined 字段，保证和“调用参数”视图一样是干净结构，避免噪声键影响阅读。
  const sessionConfig = useMemo(() => {
    const rawConfig: Record<string, unknown> = {
      subRunId,
      profile: title,
      storageMode: startMessage?.storageMode,
      childSessionId: startMessage?.childSessionId ?? finishMessage?.childSessionId,
      resolvedOverrides: startMessage?.resolvedOverrides,
      resolvedLimits: startMessage?.resolvedLimits,
    };
    const cleanEntries = Object.entries(rawConfig).filter(
      ([, value]) => value !== undefined && value !== null
    );
    return Object.fromEntries(cleanEntries);
  }, [finishMessage?.childSessionId, startMessage, subRunId, title]);

  const sessionConfigSummary = `Object (${Object.keys(sessionConfig).length})`;

  const streamFingerprint = useMemo(
    () => bodyMessages.map((message) => buildMessageFingerprint(message)).join('|'),
    [bodyMessages]
  );
  const shouldAutoOpen = !userInteracted;

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

  // 修复嵌套滚动问题：当 streamBody 滚动到边界时，阻止默认滚轮行为，让外层 MessageList 能正常滚动
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
        <span className={styles.summaryText}>子会话 {title}</span>
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
        {isBackgroundRunning && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>后台状态</div>
            <div className={styles.runningCard}>
              <div className={styles.resultSummary}>后台子会话已启动，可点击查看实时流。</div>
              <div className={styles.runningActions}>
                <span className={styles.runningHint}>
                  子 Agent 会继续把回复和思考流式回传到这里。
                </span>
                {sessionId && (
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
              {cancelError && <div className={styles.resultError}>{cancelError}</div>}
            </div>
          </div>
        )}

        <div className={styles.section}>
          <div className={styles.sectionLabel}>调用参数</div>
          <ToolJsonView value={sessionConfig} summary={sessionConfigSummary} />
        </div>

        <details className={styles.streamSection} open>
          <summary className={styles.streamSummary}>
            <span>子会话流</span>
            <span className={styles.streamCount}>{bodyMessages.length} 条消息</span>
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
          <div ref={streamRef} className={styles.streamBody} onScroll={updateStreamStickiness}>
            {bodyMessages.length === 0 ? (
              <div className={styles.streamEmpty}>等待子会话输出...</div>
            ) : (
              bodyMessages.map((groupMessage, groupIndex) =>
                renderMessageRow(groupMessage, groupIndex > 0 ? bodyMessages[groupIndex - 1] : null)
              )
            )}
          </div>
        </details>

        {finishMessage && resultHandoff && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>传递给主会话</div>
            <div className={styles.resultCard}>
              <div className={styles.resultSummary}>{resultSummary}</div>
              {visibleFindings.length > 0 && (
                <ul className={styles.resultList}>
                  {visibleFindings.map((finding, index) => (
                    <li key={`${subRunId}-finding-${index}`}>{finding}</li>
                  ))}
                </ul>
              )}
              {resultHandoff.artifacts.length > 0 && (
                <div className={styles.resultArtifacts}>
                  {resultHandoff.artifacts.map((artifact) => (
                    <span key={`${artifact.kind}-${artifact.id}`} className={styles.artifactPill}>
                      {artifact.label}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}

        {finishMessage && resultFailure && (
          <div className={styles.section}>
            <div className={styles.sectionLabel}>子会话失败</div>
            <div className={styles.resultCard}>
              <div className={styles.resultSummary}>{resultFailure.displayMessage}</div>
              {resultFailure.technicalMessage && (
                <details className={styles.streamSection}>
                  <summary className={styles.streamSummary}>
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
                  <div className={styles.streamBody}>
                    <div className={styles.resultError}>{resultFailure.technicalMessage}</div>
                  </div>
                </details>
              )}
            </div>
          </div>
        )}
      </div>
    </details>
  );
}

export default memo(SubRunBlock);
