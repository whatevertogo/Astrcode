import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

interface RunInfo {
  port?: number;
  token?: string;
}

const APP_HOME_OVERRIDE_ENV = 'ASTRCODE_HOME_DIR';

function resolveAstrcodeHomeDir(): string {
  const overriddenHome = process.env[APP_HOME_OVERRIDE_ENV]?.trim();
  if (overriddenHome) {
    return overriddenHome;
  }
  return os.homedir();
}

function resolveApiProxyTarget(): string | undefined {
  const runInfoPath = path.join(resolveAstrcodeHomeDir(), '.astrcode', 'run.json');
  if (!fs.existsSync(runInfoPath)) {
    return undefined;
  }

  const raw = fs.readFileSync(runInfoPath, 'utf8');
  const runInfo = JSON.parse(raw) as RunInfo;
  if (!runInfo.port) {
    return undefined;
  }
  return `http://127.0.0.1:${runInfo.port}`;
}

function resolveApiProxyHeaders(): Record<string, string> | undefined {
  const runInfoPath = path.join(resolveAstrcodeHomeDir(), '.astrcode', 'run.json');
  if (!fs.existsSync(runInfoPath)) {
    return undefined;
  }

  const raw = fs.readFileSync(runInfoPath, 'utf8');
  const runInfo = JSON.parse(raw) as RunInfo;
  const token = runInfo.token?.trim();
  if (!token) {
    return undefined;
  }

  return {
    'x-astrcode-token': token,
  };
}

const apiProxyTarget = resolveApiProxyTarget();
const apiProxyHeaders = resolveApiProxyHeaders();

export default defineConfig({
  plugins: [react()],
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
