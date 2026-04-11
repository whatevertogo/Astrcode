interface TopBarProps {
  projectName: string | null;
  sessionTitle: string | null;
  activeSubRunPath: string[];
  activeSubRunBreadcrumbs: Array<{ subRunId: string; title: string }>;
  isSidebarOpen: boolean;
  toggleSidebar: () => void;
  onCloseSubRun: () => void;
  onNavigateSubRunPath: (subRunPath: string[]) => void;
}

export default function TopBar({
  projectName,
  sessionTitle,
  activeSubRunPath,
  activeSubRunBreadcrumbs,
  isSidebarOpen,
  toggleSidebar,
  onCloseSubRun,
  onNavigateSubRunPath,
}: TopBarProps) {
  return (
    <div className="relative z-30 flex items-center justify-between gap-4 px-[22px] py-3.5 border-b border-border bg-[linear-gradient(180deg,rgba(255,252,247,0.96)_0%,rgba(252,250,247,0.9)_100%)] backdrop-blur-[12px] flex-shrink-0 max-[899px]:flex-wrap max-[899px]:px-4">
      <div className="flex-1 flex items-center gap-1.5 min-w-0">
        <button
          className="flex items-center justify-center bg-transparent border-none text-text-secondary rounded-md p-1 -ml-1 transition-all duration-200 ease-out cursor-pointer outline-none hover:bg-black/5 hover:text-text-primary"
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
            <span className="text-[13px] text-text-secondary overflow-hidden text-ellipsis whitespace-nowrap">
              {projectName}
            </span>
            {sessionTitle && (
              <>
                <span className="text-text-faint text-[13px]">›</span>
                {activeSubRunPath.length > 0 ? (
                  <button
                    type="button"
                    className="inline-flex items-center justify-center min-h-[26px] px-2 border border-border rounded-full bg-surface text-text-secondary text-xs font-medium cursor-pointer transition-[background,border-color,color] duration-150 ease-out hover:bg-black/3 hover:border-black/12 hover:text-text-primary"
                    onClick={onCloseSubRun}
                  >
                    {sessionTitle}
                  </button>
                ) : (
                  <span className="text-[13px] text-text-primary font-medium overflow-hidden text-ellipsis whitespace-nowrap">
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
                  className="inline-flex items-center gap-1.5 min-w-0"
                >
                  <span className="text-text-faint text-[13px]">›</span>
                  {isLast ? (
                    <span className="text-[13px] text-text-primary font-semibold overflow-hidden text-ellipsis whitespace-nowrap">
                      {breadcrumb.title}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="inline-flex items-center justify-center min-h-[26px] px-2.5 border border-border rounded-full bg-surface text-text-secondary text-xs font-semibold cursor-pointer transition-[background,border-color,color] duration-150 ease-out hover:bg-black/3 hover:border-black/12 hover:text-text-primary"
                      onClick={() => onNavigateSubRunPath(activeSubRunPath.slice(0, index + 1))}
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
    </div>
  );
}
