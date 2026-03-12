import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import react from '@vitejs/plugin-react';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

interface RunInfo {
  port?: number;
  token?: string;
  pid?: number;
}

interface BrowserBootstrapPayload {
  serverOrigin: string;
  token: string;
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

function resolveApiProxyHeaders(): Record<string, string> | undefined {
  const runInfo = readRunInfo();
  const token = runInfo?.token?.trim();
  if (!token) {
    return undefined;
  }

  return {
    'x-astrcode-token': token,
  };
}

function resolveBrowserBootstrapPayload(): BrowserBootstrapPayload | null {
  const runInfo = readRunInfo();
  const token = runInfo?.token?.trim();
  if (!runInfo?.port || !token) {
    return null;
  }

  return {
    serverOrigin: `http://127.0.0.1:${runInfo.port}`,
    token,
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
const apiProxyHeaders = resolveApiProxyHeaders();

export default defineConfig({
  plugins: [react(), astrcodeBrowserBootstrapPlugin()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: apiProxyTarget
      ? {
          '/api': {
            target: apiProxyTarget,
            changeOrigin: false,
            headers: apiProxyHeaders,
          },
        }
      : undefined,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    clearMocks: true,
  },
});
