import React, { Component } from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
import App from './App';
import { logger } from './lib/logger';

interface RootErrorBoundaryState {
  hasError: boolean;
  message: string;
}

class RootErrorBoundary extends Component<{ children: React.ReactNode }, RootErrorBoundaryState> {
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
    logger.error('main', 'root render failed', { error, errorInfo });
  }

  override render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen bg-terminal-bg-from px-6 py-6 font-mono text-terminal-error">
          <h1 className="mt-0 text-[18px]">AstrCode 前端渲染崩溃</h1>
          <p className="text-danger-soft">
            渲染错误已被拦截，窗口没有退出。请查看 DevTools Console。
          </p>
          <pre className="m-0 whitespace-pre-wrap overflow-wrap-anywhere rounded-lg border border-danger bg-black/20 p-3">
            {this.state.message}
          </pre>
        </div>
      );
    }

    return this.props.children;
  }
}

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Root element "#root" not found');
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <App />
    </RootErrorBoundary>
  </React.StrictMode>
);
