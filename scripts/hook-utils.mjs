import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export const repoRoot = resolve(scriptDir, '..');
export const isWindows = process.platform === 'win32';
export const textDecoder = new TextDecoder('utf8');

export function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    shell: isWindows,
    ...options,
  }).trim();
}

export function runBuffer(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: null,
    stdio: ['ignore', 'pipe', 'pipe'],
    shell: isWindows,
    ...options,
  });
}

export function runWithInheritedOutput(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: isWindows,
    ...options,
  });
}

export function unique(items) {
  return [...new Set(items)];
}

export function listStagedFiles() {
  return run('git', ['diff', '--cached', '--name-only', '--diff-filter=ACMR'])
    .split(/\r?\n/u)
    .filter(Boolean);
}

export function listFilesWithUnstagedChanges(files) {
  if (files.length === 0) {
    return [];
  }

  return run('git', ['diff', '--name-only', '--', ...files])
    .split(/\r?\n/u)
    .filter(Boolean);
}

export function readStagedBlob(path) {
  return runBuffer('git', ['show', `:${path}`]);
}

export function stagedBlobSize(path) {
  return Number(run('git', ['cat-file', '-s', `:${path}`]));
}

export function isProbablyBinary(buffer) {
  return buffer.includes(0);
}
