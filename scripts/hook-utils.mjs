import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));

export const repoRoot = resolve(scriptDir, '..');
export const isWindows = process.platform === 'win32';
export const textDecoder = new TextDecoder('utf8');
const windowsShellCommands = new Set(['npm', 'npx', 'npm.cmd', 'npx.cmd']);

function escapeCmdArgument(argument) {
  if (argument.length === 0) {
    return '""';
  }

  const escaped = argument
    .replace(/\^/gu, '^^')
    .replace(/"/gu, '^"')
    .replace(/([&|<>()!])/gu, '^$1');

  if (/[\s&|<>()!"^]/u.test(argument)) {
    return `"${escaped}"`;
  }

  return escaped;
}

function buildExecution(command, args) {
  if (!isWindows || !windowsShellCommands.has(command)) {
    return { file: command, args };
  }

  const comspec = process.env.ComSpec ?? 'cmd.exe';
  const cmdCommand = [command, ...args].map(escapeCmdArgument).join(' ');
  return {
    file: comspec,
    args: ['/d', '/s', '/c', cmdCommand],
  };
}

export function run(command, args, options = {}) {
  const execution = buildExecution(command, args);
  return execFileSync(execution.file, execution.args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    ...options,
  }).trim();
}

export function runBuffer(command, args, options = {}) {
  const execution = buildExecution(command, args);
  return execFileSync(execution.file, execution.args, {
    cwd: repoRoot,
    encoding: null,
    stdio: ['ignore', 'pipe', 'pipe'],
    ...options,
  });
}

export function runWithInheritedOutput(command, args, options = {}) {
  const execution = buildExecution(command, args);
  execFileSync(execution.file, execution.args, {
    cwd: repoRoot,
    stdio: 'inherit',
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
