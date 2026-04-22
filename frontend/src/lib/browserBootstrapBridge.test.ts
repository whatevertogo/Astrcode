import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

const APP_HOME_OVERRIDE_ENV = 'ASTRCODE_HOME_DIR';

describe('vite browser bootstrap bridge', () => {
  let tempHomeDir: string | null = null;

  function mockProcessIdentity(identity: string, status = 0): void {
    vi.doMock('node:child_process', () => ({
      spawnSync: vi.fn(() => ({
        status,
        stdout: identity,
      })),
    }));
  }

  afterEach(() => {
    vi.resetModules();
    vi.unmock('node:child_process');
    if (tempHomeDir) {
      fs.rmSync(tempHomeDir, { recursive: true, force: true });
      tempHomeDir = null;
    }
    delete process.env[APP_HOME_OVERRIDE_ENV];
  });

  it('returns the live server origin together with the bootstrap token', async () => {
    mockProcessIdentity('astrcode-server');
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

  it('returns null when run info points to a dead pid', async () => {
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'stale-bootstrap-token',
        pid: 999_999,
        expiresAtMs: Date.now() + 60_000,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });

  it('returns null when run info json is malformed', async () => {
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(path.join(runInfoDir, 'run.json'), '{not-valid-json');

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });

  it('returns null when run info omits pid even if other fields look valid', async () => {
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'pid-less-bootstrap-token',
        expiresAtMs: Date.now() + 60_000,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });

  it('returns null when run info bootstrap token has expired', async () => {
    mockProcessIdentity('astrcode-server');
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'expired-bootstrap-token',
        pid: process.pid,
        expiresAtMs: 1,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });

  it('returns null when run info pid belongs to a live non-server process', async () => {
    mockProcessIdentity('node');
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'fake-live-pid-token',
        pid: process.pid,
        expiresAtMs: Date.now() + 60_000,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });

  it('returns null when process identity lookup fails for an otherwise live pid', async () => {
    mockProcessIdentity('', 1);
    tempHomeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'astrcode-vite-'));
    process.env[APP_HOME_OVERRIDE_ENV] = tempHomeDir;

    const runInfoDir = path.join(tempHomeDir, '.astrcode');
    fs.mkdirSync(runInfoDir, { recursive: true });
    fs.writeFileSync(
      path.join(runInfoDir, 'run.json'),
      JSON.stringify({
        port: 62000,
        token: 'identity-lookup-failed-token',
        pid: process.pid,
        expiresAtMs: Date.now() + 60_000,
      })
    );

    const { resolveBrowserBootstrapPayload } = await import('../../vite.config');

    expect(resolveBrowserBootstrapPayload()).toBeNull();
  });
});
