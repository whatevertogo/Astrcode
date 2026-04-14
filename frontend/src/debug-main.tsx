async function bootstrapDebugWorkbench(): Promise<void> {
  const rootElement = document.getElementById('root');
  if (!rootElement) {
    throw new Error('Root element "#root" not found');
  }

  const renderFatalError = (message: string) => {
    rootElement.innerHTML = `
      <div class="min-h-screen bg-terminal-bg-from px-6 py-6 font-mono text-terminal-error">
        <h1 class="mt-0 text-[18px]">Debug Workbench 启动失败</h1>
        <p class="text-danger-soft">
          调试窗口没有成功挂载。请查看 DevTools Console 或桌面端日志。
        </p>
        <pre class="m-0 overflow-wrap-anywhere rounded-lg border border-danger bg-black/20 p-3 whitespace-pre-wrap">${escapeHtml(message)}</pre>
      </div>
    `;
  };

  const attachGlobalErrorHandlers = () => {
    window.addEventListener('error', (event) => {
      const message =
        event.error instanceof Error
          ? `${event.error.name}: ${event.error.message}`
          : event.message || 'unknown window error';
      renderFatalError(message);
    });

    window.addEventListener('unhandledrejection', (event) => {
      const reason: unknown = event.reason;
      const message =
        reason instanceof Error ? `${reason.name}: ${reason.message}` : String(reason);
      renderFatalError(message);
    });
  };

  attachGlobalErrorHandlers();

  try {
    await import('./index.css');
    const React = await import('react');
    const ReactDOM = await import('react-dom/client');
    const [{ default: DebugWorkbenchApp }, { logger }] = await Promise.all([
      import('./DebugWorkbenchApp'),
      import('./lib/logger'),
    ]);

    interface RootErrorBoundaryState {
      hasError: boolean;
      message: string;
    }

    class RootErrorBoundary extends React.Component<
      { children: React.ReactNode },
      RootErrorBoundaryState
    > {
      state: RootErrorBoundaryState = {
        hasError: false,
        message: '',
      };

      static getDerivedStateFromError(error: Error): RootErrorBoundaryState {
        return {
          hasError: true,
          message: error.message || String(error),
        };
      }

      override componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
        logger.error('debug-main', 'debug workbench render failed', { error, errorInfo });
      }

      override render() {
        if (this.state.hasError) {
          return (
            <div className="min-h-screen bg-terminal-bg-from px-6 py-6 font-mono text-terminal-error">
              <h1 className="mt-0 text-[18px]">Debug Workbench 渲染崩溃</h1>
              <p className="text-danger-soft">
                渲染错误已被拦截，窗口没有退出。请查看 DevTools Console。
              </p>
              <pre className="m-0 overflow-wrap-anywhere rounded-lg border border-danger bg-black/20 p-3 whitespace-pre-wrap">
                {this.state.message}
              </pre>
            </div>
          );
        }

        return this.props.children;
      }
    }

    ReactDOM.createRoot(rootElement).render(
      <React.StrictMode>
        <RootErrorBoundary>
          <DebugWorkbenchApp />
        </RootErrorBoundary>
      </React.StrictMode>
    );
  } catch (error) {
    const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
    renderFatalError(message);
    throw error;
  }
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

void bootstrapDebugWorkbench();
