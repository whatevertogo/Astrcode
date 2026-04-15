import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  ChildSessionNotificationMessage,
  ParentDelivery,
  SubRunFinishMessage,
  SubRunStartMessage,
} from '../../types';
import type { ThreadItem } from '../../lib/subRunView';
import { cn } from '../../lib/utils';
import {
  chevronIcon,
  expandableBody,
  infoButton,
  pillDanger,
  pillNeutral,
  pillSuccess,
  pillWarning,
  subtleActionButton,
} from '../../lib/styles';

interface SubRunBlockProps {
  subRunId: string;
  sessionId: string | null;
  childSessionId?: string;
  title: string;
  startMessage?: SubRunStartMessage;
  finishMessage?: SubRunFinishMessage;
  latestNotification?: ChildSessionNotificationMessage;
  threadItems: ThreadItem[];
  streamFingerprint: string;
  hasDescriptorLineage: boolean;
  renderThreadItems: (
    items: ThreadItem[],
    options?: {
      nested?: boolean;
    }
  ) => React.ReactNode[];
  onCancelSubRun: (sessionId: string, agentId: string) => void | Promise<void>;
  onFocusSubRun?: (subRunId: string) => void | Promise<void>;
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

function getStorageModeLabel(startMessage?: SubRunStartMessage, childSessionId?: string): string {
  const storageMode = startMessage?.resolvedOverrides.storageMode ?? startMessage?.storageMode;
  return storageMode === 'independentSession' || childSessionId
    ? 'independent session'
    : 'independent session';
}

function getStatusVariant(status: SubRunStatus): string {
  switch (status) {
    case 'completed':
      return pillSuccess;
    case 'aborted':
      return pillNeutral;
    case 'token_exceeded':
      return pillWarning;
    case 'failed':
      return pillDanger;
    case 'running':
    default:
      return pillNeutral;
  }
}

function isVisibleActivityItem(item: ThreadItem): boolean {
  if (item.kind === 'subRun') {
    return true;
  }

  return item.message.kind === 'assistant' || item.message.kind === 'toolCall';
}

function getDeliveryMessage(delivery?: ParentDelivery): string | null {
  const message = delivery?.payload.message?.trim();
  return message && message.length > 0 ? message : null;
}

function SubRunBlock({
  subRunId,
  sessionId,
  childSessionId: projectedChildSessionId,
  title,
  startMessage,
  finishMessage,
  latestNotification,
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
  const childSessionId =
    projectedChildSessionId ?? startMessage?.childSessionId ?? finishMessage?.childSessionId;
  const metrics =
    finishMessage !== undefined
      ? `${finishMessage.stepCount} steps`
      : getStorageModeLabel(startMessage, childSessionId);
  const resultHandoff = finishMessage?.result.handoff;
  const resultFailure = finishMessage?.result.failure;
  const latestDelivery = latestNotification?.delivery ?? resultHandoff?.delivery;
  const latestDeliveryMessage = getDeliveryMessage(latestDelivery);
  const isBackgroundRunning = status === 'running';
  const navigationLabel =
    childSessionId !== undefined
      ? '打开独立会话'
      : displayMode === 'directory'
        ? '进入子执行'
        : '查看子执行';
  const activityItems = useMemo(() => threadItems.filter(isVisibleActivityItem), [threadItems]);
  const activitySummary =
    resultFailure?.displayMessage ||
    latestDeliveryMessage ||
    (isBackgroundRunning
      ? childSessionId
        ? '独立子会话正在后台运行，请打开会话查看实时输出。'
        : '独立子会话正在初始化；会话入口可用后即可直接打开。'
      : childSessionId
        ? '这是独立子会话，请打开会话查看完整输出。'
        : '这是独立子会话；如果还没有会话入口，请稍后再查看。');
  const shouldAutoOpen = !userInteracted && isBackgroundRunning;
  const cancelTargetAgentId = startMessage?.agentId ?? subRunId;

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
      // 使用 agentId 定位，fallback 到 subRunId（旧事件可能缺少 agentId）
      await onCancelSubRun(sessionId, cancelTargetAgentId);
    } catch (error) {
      setCancelError(error instanceof Error ? error.message : String(error));
    } finally {
      setCancelling(false);
    }
  }, [cancelTargetAgentId, cancelling, onCancelSubRun, sessionId]);

  const handleOpenView = useCallback(async () => {
    if (childSessionId) {
      await onOpenChildSession?.(childSessionId);
      return;
    }
    await onFocusSubRun?.(subRunId);
  }, [childSessionId, onFocusSubRun, onOpenChildSession, subRunId]);

  const renderToolbar = () => (
    <div className="flex items-start justify-between gap-3 flex-wrap">
      <div className="flex-1 basis-[260px] min-w-0 text-[13px] leading-relaxed text-text-secondary whitespace-pre-wrap overflow-wrap-anywhere">
        {activitySummary}
      </div>
      <div className="flex items-center gap-2.5 flex-wrap">
        {(onFocusSubRun || (childSessionId && onOpenChildSession)) && (
          <button
            type="button"
            className={cn(infoButton, 'min-h-[30px]')}
            onClick={() => void handleOpenView()}
          >
            {navigationLabel}
          </button>
        )}
        {sessionId && isBackgroundRunning && (
          <button
            type="button"
            className={cn(
              subtleActionButton,
              'min-h-[30px] disabled:cursor-wait disabled:opacity-60'
            )}
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
      className="m-0 bg-transparent border-none group"
      open={isBackgroundRunning || activityItems.length > 0}
    >
      <summary className="inline-flex items-center gap-2 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none text-text-secondary transition-opacity duration-150 ease-out text-sm font-medium list-none [&::-webkit-details-marker]:hidden hover:opacity-80">
        <span>思考与工具</span>
        <span className="text-xs text-text-secondary opacity-80">
          {activityItems.length > 0 ? `${activityItems.length} 条活动` : '等待输出'}
        </span>
        <span className={chevronIcon}>
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
      <div
        ref={streamRef}
        className="mt-2 mb-2 max-h-[360px] max-sm:max-h-[300px] overflow-y-auto pr-1 flex flex-col gap-3.5"
        onScroll={updateStreamStickiness}
      >
        {activityItems.length === 0 ? (
          <div className="py-1 text-text-secondary text-xs leading-relaxed">
            {childSessionId
              ? isBackgroundRunning
                ? '该子 Agent 运行在独立会话中；请点击"打开独立会话"查看实时输出。'
                : '该子 Agent 的完整输出保存在独立会话中；请点击"打开独立会话"查看。'
              : isBackgroundRunning
                ? '独立子会话正在初始化；会话入口就绪后可直接打开查看。'
                : '该子执行没有生成可直接展示的内联输出。'}
          </div>
        ) : (
          renderThreadItems(activityItems, { nested: true })
        )}
      </div>
    </details>
  );

  const renderFinalReply = () => {
    // 成功交付时展示最终回复摘要
    // 独立子会话的完整结果应该留在子会话里，父视图只保留摘要和入口。
    const completedDelivery = latestDelivery?.kind === 'completed' ? latestDelivery : undefined;
    const completedSummary = getDeliveryMessage(completedDelivery);
    const completedFindings =
      completedDelivery?.kind === 'completed'
        ? completedDelivery.payload.findings
        : (resultHandoff?.findings ?? []);

    if (!completedSummary || status !== 'completed' || childSessionId) {
      return null;
    }
    return (
      <details className="m-0 bg-transparent border-none group" open>
        <summary className="inline-flex items-center gap-2 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none text-text-secondary transition-opacity duration-150 ease-out text-sm font-medium list-none [&::-webkit-details-marker]:hidden hover:opacity-80">
          <span>最终回复</span>
          <span className={chevronIcon}>
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
        <div className="mt-2 mb-2">
          <div className="flex-1 basis-[260px] min-w-0 text-[13px] leading-relaxed text-text-secondary whitespace-pre-wrap overflow-wrap-anywhere">
            {completedSummary}
          </div>
          {completedFindings.length > 0 && (
            <ul className="mt-1 mb-0 ml-4 p-0 list-disc text-text-secondary text-[0.85em]">
              {completedFindings.map((finding, index) => (
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
      className="group mb-1.5 ml-[var(--chat-assistant-content-offset)] block min-w-0 max-w-full animate-block-enter motion-reduce:animate-none"
      onToggle={(event) => {
        if (event.target === event.currentTarget && event.nativeEvent.isTrusted) {
          setUserInteracted(true);
        }
      }}
    >
      <summary
        className="flex items-center gap-2 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none text-text-secondary transition-opacity duration-150 ease-out text-[13px] font-normal font-mono list-none flex-nowrap w-full min-w-0 [&::-webkit-details-marker]:hidden hover:opacity-80"
        title={`${title} · ${statusLabel} · ${metrics}`}
      >
        <span className="block flex-1 min-w-0 whitespace-nowrap overflow-hidden text-ellipsis">
          子 Agent {title}
        </span>
        <span className={getStatusVariant(status)}>{statusLabel}</span>
        <span className="shrink-0 text-xs text-text-secondary whitespace-nowrap max-sm:hidden">
          {metrics}
        </span>
        <span className={chevronIcon}>
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

      <div
        className={cn(expandableBody, 'min-w-0 max-w-full overflow-x-hidden flex flex-col gap-3')}
      >
        {displayMode === 'directory' ? (
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div className="flex-1 basis-[260px] min-w-0 text-[13px] leading-relaxed text-text-secondary whitespace-pre-wrap overflow-wrap-anywhere">
              {activitySummary}
            </div>
            {(onFocusSubRun || (childSessionId && onOpenChildSession)) && (
              <button
                type="button"
                className={cn(infoButton, 'min-h-[30px]')}
                onClick={() => void handleOpenView()}
              >
                {navigationLabel}
              </button>
            )}
          </div>
        ) : (
          <>
            {renderToolbar()}
            {cancelError && (
              <div className="text-danger text-xs leading-relaxed font-mono whitespace-pre-wrap overflow-wrap-anywhere">
                {cancelError}
              </div>
            )}
            {resultFailure && (
              <div className="flex flex-col gap-2">
                <div className="text-xs font-semibold text-danger">执行失败</div>
                <div className="text-[13px] leading-relaxed text-text-primary whitespace-pre-wrap overflow-wrap-anywhere">
                  {resultFailure.displayMessage}
                </div>
                {resultFailure.technicalMessage && (
                  <details className="m-0 bg-transparent border-none group">
                    <summary className="inline-flex items-center gap-2 py-1 min-h-[24px] cursor-pointer select-none bg-transparent border-none text-text-secondary transition-opacity duration-150 ease-out text-sm font-medium list-none [&::-webkit-details-marker]:hidden hover:opacity-80">
                      <span>技术详情</span>
                      <span className={chevronIcon}>
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
                    <div className="mt-2 mb-2">
                      <div className="text-danger text-xs leading-relaxed font-mono whitespace-pre-wrap overflow-wrap-anywhere">
                        {resultFailure.technicalMessage}
                      </div>
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
