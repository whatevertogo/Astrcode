import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
const hooksPath = '.githooks';
const preCommitHookPath = resolve(repoRoot, hooksPath, 'pre-commit');

function runGit(args) {
  return execFileSync('git', args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

if (process.env.CI === 'true') {
  // CI checkout is ephemeral, so mutating local git config only adds noise.
  console.log('Skipping git hook installation in CI.');
  process.exit(0);
}

if (!existsSync(preCommitHookPath)) {
  // Fail open so dependency installation still succeeds even if the hook file is missing.
  console.warn('Skipping git hook installation because .githooks/pre-commit is missing.');
  process.exit(0);
}

try {
  const gitRoot = resolve(runGit(['rev-parse', '--show-toplevel']));
  if (gitRoot !== repoRoot) {
    // Only touch git config when this script is executed from the intended repository root.
    console.log(`Skipping git hook installation because git root is ${gitRoot}.`);
    process.exit(0);
  }
} catch {
  // Installing dependencies from a source archive should not fail just because git metadata is absent.
  console.log('Skipping git hook installation because this directory is not a git checkout.');
  process.exit(0);
}

try {
  // The tracked hook is stored in the repo, but Unix users still need the executable bit locally.
  chmodSync(preCommitHookPath, 0o755);
} catch {
  // Windows ignores chmod and some filesystems reject it, so keep the installation best-effort.
}

let currentHooksPath = '';

try {
  currentHooksPath = runGit(['config', '--get', 'core.hooksPath']);
} catch {
  currentHooksPath = '';
}

if (currentHooksPath && currentHooksPath !== hooksPath) {
  // Respect an explicit custom hook path instead of silently overwriting local repository config.
  console.warn(
    `Skipping git hook installation because core.hooksPath is already set to "${currentHooksPath}".`,
  );
  process.exit(0);
}

if (currentHooksPath === hooksPath) {
  console.log('Git hooks are already configured.');
  process.exit(0);
}

runGit(['config', 'core.hooksPath', hooksPath]);
console.log(`Configured git hooks to use ${hooksPath}.`);
