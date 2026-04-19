import { useMemo, useState } from 'react';
import { cn } from '../../lib/utils';
import { useChatScreenContext } from './ChatScreenContext';

function statusLabel(status: 'pending' | 'in_progress' | 'completed'): string {
  switch (status) {
    case 'in_progress':
      return '进行中';
    case 'completed':
      return '已完成';
    case 'pending':
    default:
      return '待处理';
  }
}

function statusClass(status: 'pending' | 'in_progress' | 'completed'): string {
  switch (status) {
    case 'in_progress':
      return 'border-amber-300/60 bg-amber-100/80 text-amber-900';
    case 'completed':
      return 'border-emerald-300/60 bg-emerald-100/80 text-emerald-900';
    case 'pending':
    default:
      return 'border-slate-300/70 bg-slate-100/80 text-slate-700';
  }
}

export default function TaskPanel() {
  const { conversationControl } = useChatScreenContext();
  // TODO(task-panel): 当前 UI 假设 control.activeTasks 只代表一个 owner 的任务快照。
  // 后续支持多 owner 并行展示时，需要把这里重构成按 owner 分组的多卡片/多分区布局，
  // 同时保留每个 owner 自己的 in_progress 与统计摘要。
  const tasks = conversationControl?.activeTasks;
  const [collapsed, setCollapsed] = useState(false);

  const summary = useMemo(() => {
    if (!tasks || tasks.length === 0) {
      return null;
    }
    const inProgress = tasks.find((task) => task.status === 'in_progress');
    const pendingCount = tasks.filter((task) => task.status === 'pending').length;
    const completedCount = tasks.filter((task) => task.status === 'completed').length;
    return {
      inProgress,
      pendingCount,
      completedCount,
      total: tasks.length,
    };
  }, [tasks]);

  if (!tasks || tasks.length === 0 || !summary) {
    return null;
  }

  return (
    <section className="shrink-0 border-b border-border bg-[linear-gradient(180deg,rgba(251,249,244,0.96)_0%,rgba(246,241,231,0.94)_100%)] px-[var(--chat-content-horizontal-padding)] py-3.5 max-sm:px-[var(--chat-content-horizontal-padding-mobile)]">
      <div className="mx-auto flex w-[min(100%,var(--chat-content-max-width))] flex-col overflow-hidden rounded-[20px] border border-black/8 bg-white/78 shadow-[0_16px_34px_rgba(88,72,36,0.08)] backdrop-blur-[10px]">
        <button
          type="button"
          className="flex w-full items-start justify-between gap-4 px-4 py-3 text-left transition-[background-color] duration-150 ease-out hover:bg-black/[0.02]"
          onClick={() => setCollapsed((value) => !value)}
          aria-expanded={!collapsed}
        >
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="inline-flex min-h-[24px] items-center rounded-full border border-black/8 bg-[#f4ede0] px-2.5 text-[11px] font-bold tracking-[0.06em] text-[#6f5730]">
                TASKS
              </span>
              <span className="text-xs text-text-secondary">
                {summary.inProgress
                  ? `当前执行 · ${summary.inProgress.activeForm ?? summary.inProgress.content}`
                  : '当前没有进行中的任务'}
              </span>
            </div>
            <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[12px] text-text-secondary">
              <span>待处理 {summary.pendingCount}</span>
              <span>已完成 {summary.completedCount}</span>
              <span>总计 {summary.total}</span>
            </div>
          </div>
          <span
            className={cn(
              'inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-black/8 bg-white/80 text-text-secondary transition-transform duration-150 ease-out',
              collapsed ? '' : 'rotate-180'
            )}
            aria-hidden="true"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </span>
        </button>
        {!collapsed ? (
          <div className="border-t border-black/6 px-4 py-3.5">
            <div className="flex flex-col gap-2.5">
              {tasks.map((task, index) => (
                <div
                  key={`${task.status}:${task.content}:${index}`}
                  className="flex items-start justify-between gap-3 rounded-2xl border border-black/6 bg-[rgba(255,252,246,0.9)] px-3.5 py-3"
                >
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium leading-5 text-text-primary">
                      {task.content}
                    </div>
                    {task.activeForm ? (
                      <div className="mt-1 text-xs leading-5 text-text-secondary">
                        {task.activeForm}
                      </div>
                    ) : null}
                  </div>
                  <span
                    className={cn(
                      'inline-flex min-h-[24px] shrink-0 items-center rounded-full border px-2.5 text-[11px] font-semibold',
                      statusClass(task.status)
                    )}
                  >
                    {statusLabel(task.status)}
                  </span>
                </div>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </section>
  );
}
