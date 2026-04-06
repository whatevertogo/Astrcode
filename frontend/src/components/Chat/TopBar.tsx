import styles from './TopBar.module.css';

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
    <div className={styles.topBar}>
      <div className={styles.breadcrumb}>
        <button
          className={styles.toggleSidebarBtn}
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
            <span className={styles.projectName}>{projectName}</span>
            {sessionTitle && (
              <>
                <span className={styles.sep}>›</span>
                {activeSubRunPath.length > 0 ? (
                  <button type="button" className={styles.breadcrumbButton} onClick={onCloseSubRun}>
                    {sessionTitle}
                  </button>
                ) : (
                  <span className={styles.sessionTitle}>{sessionTitle}</span>
                )}
              </>
            )}
            {activeSubRunBreadcrumbs.map((breadcrumb, index) => {
              const isLast = index === activeSubRunBreadcrumbs.length - 1;
              return (
                <span key={breadcrumb.subRunId} className={styles.breadcrumbSegment}>
                  <span className={styles.sep}>›</span>
                  {isLast ? (
                    <span className={styles.subRunTitle}>{breadcrumb.title}</span>
                  ) : (
                    <button
                      type="button"
                      className={styles.subRunBackBtn}
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
          <span className={styles.empty}>未选择会话</span>
        )}
      </div>
    </div>
  );
}
