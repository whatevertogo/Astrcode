import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

const APP_HOME_OVERRIDE_ENV = 'ASTRCODE_HOME_DIR';

describe('vite browser bootstrap bridge', () => {
  let tempHomeDir: string | null = null;

  afterEach(() => {
    vi.resetModules();
    if (tempHomeDir) {
      fs.rmSync(tempHomeDir, { recursive: true, force: true });
      tempHomeDir = null;
    }
    delete process.env[APP_HOME_OVERRIDE_ENV];
  });

  it('returns the live server origin together with the bootstrap token', async () => {
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'bootstrap-token',
        pid: process.pid,
        expiresAtMs: Date.now() + 60_000,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toEqual({
      token: 'bootstrap-token',
      serverOrigin: 'http://127.0.0.1:62000',
    });
  });
});
