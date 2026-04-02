import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
const hooksPath = '.githooks';
const requiredHookFiles = ['pre-commit', 'pre-push'];

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

if (requiredHookFiles.some((file) => !existsSync(resolve(repoRoot, hooksPath, file)))) {
  // Fail open so dependency installation still succeeds from source archives, but keep the warning
  // explicit because missing hook wrappers mean the repo is no longer enforcing the documented flow.
  console.warn(
    `Skipping git hook installation because one of ${requiredHookFiles
      .map((file) => `${hooksPath}/${file}`)
      .join(', ')} is missing.`,
  );
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
  // The tracked wrappers live in git, but Unix users still need the executable bit locally.
  for (const hookFile of requiredHookFiles) {
    chmodSync(resolve(repoRoot, hooksPath, hookFile), 0o755);
  }
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
