import React, { Component } from 'react';
import ReactDOM from 'react-dom/client';
import './index.css';
import App from './App';

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
    console.error('root render failed', { error, errorInfo });
  }

  override render() {
    if (this.state.hasError) {
      return (
        <div
          style={{
            minHeight: '100vh',
            background: '#1a1a1a',
            color: '#f3d6d6',
            padding: '24px',
            fontFamily: 'Consolas, monospace',
          }}
        >
          <h1 style={{ marginTop: 0, fontSize: '18px' }}>AstrCode 前端渲染崩溃</h1>
          <p style={{ color: '#d7a6a6' }}>
            渲染错误已被拦截，窗口没有退出。请查看 DevTools Console。
          </p>
          <pre
            style={{
              margin: 0,
              whiteSpace: 'pre-wrap',
              overflowWrap: 'anywhere',
              background: '#2a1d1d',
              border: '1px solid #7f3b3b',
              borderRadius: '8px',
              padding: '12px',
            }}
          >
            {this.state.message}
          </pre>
        </div>
      );
    }

    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <App />
    </RootErrorBoundary>
  </React.StrictMode>
);
