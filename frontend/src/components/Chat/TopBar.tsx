import { ghostIconButton, topBarShell } from '../../lib/styles';
import { cn } from '../../lib/utils';
import { useChatScreenContext } from './ChatScreenContext';

export default function TopBar() {
  const {
    projectName,
    sessionTitle,
    currentModeId,
    conversationControl,
    activeSubRunPath,
    activeSubRunBreadcrumbs,
    isSidebarOpen,
    toggleSidebar,
    onCloseSubRun,
    onNavigateSubRunPath,
  } = useChatScreenContext();
  const activePlan = conversationControl?.activePlan;

  return (
    <div className={topBarShell}>
      <div className="flex min-w-0 flex-1 items-center gap-1.5">
        <button
          className={cn(ghostIconButton, '-ml-1 p-1')}
          type="button"
          onClick={toggleSidebar}
          aria-label={isSidebarOpen ? '收起侧边栏' : '展开侧边栏'}
          title={isSidebarOpen ? '收起侧边栏' : '展开侧边栏'}
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
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
            <line x1="9" y1="3" x2="9" y2="21"></line>
          </svg>
        </button>
        {projectName ? (
          <>
            <span className="overflow-hidden text-ellipsis whitespace-nowrap text-[13px] text-text-secondary">
              {projectName}
            </span>
            {sessionTitle && (
              <>
                <span className="text-[13px] text-text-faint">›</span>
                {activeSubRunPath.length > 0 ? (
                  <button
                    type="button"
                    className="inline-flex min-h-[26px] items-center justify-center rounded-full border border-border bg-surface px-2 text-xs font-medium text-text-secondary transition-[background,border-color,color] duration-150 ease-out hover:border-black/12 hover:bg-black/3 hover:text-text-primary"
                    onClick={() => void onCloseSubRun()}
                  >
                    {sessionTitle}
                  </button>
                ) : (
                  <span className="overflow-hidden text-ellipsis whitespace-nowrap text-[13px] font-medium text-text-primary">
                    {sessionTitle}
                  </span>
                )}
              </>
            )}
            {activeSubRunBreadcrumbs.map((breadcrumb, index) => {
              const isLast = index === activeSubRunBreadcrumbs.length - 1;
              return (
                <span
                  key={breadcrumb.subRunId}
                  className="inline-flex min-w-0 items-center gap-1.5"
                >
                  <span className="text-[13px] text-text-faint">›</span>
                  {isLast ? (
                    <span className="overflow-hidden text-ellipsis whitespace-nowrap text-[13px] font-semibold text-text-primary">
                      {breadcrumb.title}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="inline-flex min-h-[26px] items-center justify-center rounded-full border border-border bg-surface px-2.5 text-xs font-semibold text-text-secondary transition-[background,border-color,color] duration-150 ease-out hover:border-black/12 hover:bg-black/3 hover:text-text-primary"
                      onClick={() =>
                        void onNavigateSubRunPath(activeSubRunPath.slice(0, index + 1))
                      }
                    >
                      {breadcrumb.title}
                    </button>
                  )}
                </span>
              );
            })}
          </>
        ) : (
          <span className="text-[13px] text-text-muted">未选择会话</span>
        )}
      </div>
      {conversationControl && (
        <div className="ml-3 flex shrink-0 items-center gap-2">
          {currentModeId && (
            <span className="inline-flex items-center rounded-full border border-border bg-surface px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-text-secondary">
              {currentModeId}
            </span>
          )}
          {activePlan ? (
            <span
              className="inline-flex max-w-[220px] items-center truncate rounded-full border border-emerald-300/50 bg-emerald-50 px-2.5 py-1 text-[11px] font-medium text-emerald-900"
              title={`当前计划: ${activePlan.title} (${activePlan.status})`}
            >
              当前计划 · {activePlan.title}
            </span>
          ) : null}
          {conversationControl.compacting ? (
            <span className="inline-flex items-center rounded-full border border-amber-300/50 bg-amber-100/70 px-2.5 py-1 text-[11px] font-medium text-amber-900">
              正在 compact
            </span>
          ) : conversationControl.compactPending ? (
            <span className="inline-flex items-center rounded-full border border-sky-300/50 bg-sky-100/70 px-2.5 py-1 text-[11px] font-medium text-sky-900">
              compact 已排队
            </span>
          ) : null}
        </div>
      )}
    </div>
  );
}
