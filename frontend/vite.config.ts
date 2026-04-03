import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

interface RunInfo {
  port?: number;
  token?: string;
  pid?: number;
  expiresAtMs?: number;
}

interface BrowserBootstrapPayload {
  token: string;
  serverOrigin: string;
}

const APP_HOME_OVERRIDE_ENV = 'ASTRCODE_HOME_DIR';
const BROWSER_BOOTSTRAP_PATH = '/__astrcode__/run-info';

function resolveAstrcodeHomeDir(): string {
  const overriddenHome = process.env[APP_HOME_OVERRIDE_ENV]?.trim();
  if (overriddenHome) {
    return overriddenHome;
  }
  return os.homedir();
}

function isLivePid(pid: number | undefined): boolean {
  if (!Number.isInteger(pid) || !pid || pid <= 0) {
    return false;
  }

  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === 'EPERM';
  }
}

function readRunInfo(): RunInfo | null {
  const runInfoPath = path.join(resolveAstrcodeHomeDir(), '.astrcode', 'run.json');
  if (!fs.existsSync(runInfoPath)) {
    return null;
  }

  try {
    const raw = fs.readFileSync(runInfoPath, 'utf8');
    const runInfo = JSON.parse(raw) as RunInfo;
    if (runInfo.pid !== undefined && !isLivePid(runInfo.pid)) {
      return null;
    }
    if (typeof runInfo.expiresAtMs === 'number' && Date.now() > runInfo.expiresAtMs) {
      return null;
    }
    return runInfo;
  } catch {
    return null;
  }
}

function resolveApiProxyTarget(): string | undefined {
  const runInfo = readRunInfo();
  if (!runInfo?.port) {
    return undefined;
  }
  return `http://127.0.0.1:${runInfo.port}`;
}

export function resolveBrowserBootstrapPayload(): BrowserBootstrapPayload | null {
  const runInfo = readRunInfo();
  const token = runInfo?.token?.trim();
  const serverOrigin = runInfo?.port ? `http://127.0.0.1:${runInfo.port}` : null;
  if (!token || !serverOrigin) {
    return null;
  }

  // 浏览器开发态里 server 可能晚于 Vite 启动，因此桥接必须带上真实 server origin，
  // 这样前端不需要依赖启动瞬间是否已经注册了静态代理。
  return {
    token,
    serverOrigin,
  };
}

function astrcodeBrowserBootstrapPlugin(): Plugin {
  return {
    name: 'astrcode-browser-bootstrap',
    configureServer(server) {
      server.middlewares.use(BROWSER_BOOTSTRAP_PATH, (_req, res) => {
        const payload = resolveBrowserBootstrapPayload();
        res.setHeader('Content-Type', 'application/json; charset=utf-8');
        res.setHeader('Cache-Control', 'no-store');

        if (!payload) {
          res.statusCode = 503;
          res.end(
            JSON.stringify({
              error: 'astrcode-server is not ready',
            })
          );
          return;
        }

        res.statusCode = 200;
        res.end(JSON.stringify(payload));
      });
    },
  };
}

const apiProxyTarget = resolveApiProxyTarget();
console.log(
  '[astrcode] API proxy target:',
  apiProxyTarget ??
    '(optional at startup - browser bridge will connect directly when run.json is ready)'
);
export default defineConfig({
  plugins: [react(), tailwindcss(), astrcodeBrowserBootstrapPlugin()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: apiProxyTarget
      ? {
          '/api': {
            target: apiProxyTarget,
            changeOrigin: true,
          },
        }
      : undefined,
  },
  test: {
    environment: 'node',
    clearMocks: true,
  },
});
