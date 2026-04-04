import styles from './TopBar.module.css';

interface TopBarProps {
  projectName: string | null;
  sessionTitle: string | null;
  isSidebarOpen: boolean;
  toggleSidebar: () => void;
}

export default function TopBar({
  projectName,
  sessionTitle,
  isSidebarOpen,
  toggleSidebar,
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
                <span className={styles.sessionTitle}>{sessionTitle}</span>
              </>
            )}
          </>
        ) : (
          <span className={styles.empty}>未选择会话</span>
        )}
      </div>
    </div>
  );
}
