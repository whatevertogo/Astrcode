import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
// On Windows, .cmd shims (like npx.cmd) must be spawned through a shell.
const isWindows = process.platform === 'win32';
const npxCommand = isWindows ? 'npx' : 'npx';

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    shell: isWindows,
    ...options,
  }).trim();
}

function runWithInheritedOutput(command, args) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: 'inherit',
    shell: isWindows,
  });
}

function unique(items) {
  return [...new Set(items)];
}

const stagedFiles = run('git', ['diff', '--cached', '--name-only', '--diff-filter=ACMR'])
  .split(/\r?\n/u)
  .filter(Boolean);

const stagedRustFiles = stagedFiles.filter((file) => file.endsWith('.rs'));
const stagedFrontendFiles = stagedFiles.filter((file) =>
  /^frontend\/src\/.+\.(ts|tsx|css)$/u.test(file),
);
const filesToFormat = unique([...stagedRustFiles, ...stagedFrontendFiles]);

if (filesToFormat.length === 0) {
  console.log('pre-commit: no staged Rust or frontend source files to format.');
  process.exit(0);
}

const filesWithUnstagedChanges = run('git', ['diff', '--name-only', '--', ...filesToFormat])
  .split(/\r?\n/u)
  .filter(Boolean);

if (filesWithUnstagedChanges.length > 0) {
  console.error(
    `pre-commit: refusing to format files with unstaged changes: ${filesWithUnstagedChanges.join(', ')}`,
  );
  console.error('pre-commit: stage or stash those edits first so the hook does not rewrite hidden hunks.');
  process.exit(1);
}

if (stagedRustFiles.length > 0) {
  // Restrict rustfmt to staged files so a commit only rewrites code that is already part of the changeset.
  console.log(`pre-commit: formatting ${stagedRustFiles.length} Rust file(s).`);
  runWithInheritedOutput('cargo', ['fmt', '--all', '--', ...stagedRustFiles]);

  console.log('pre-commit: running clippy on workspace...');
  runWithInheritedOutput('cargo', ['clippy', '--workspace', '--', '-D', 'warnings']);

  // Match CI's Rust test entrypoint so Windows-only regressions are blocked before push.
  console.log('pre-commit: running Rust workspace tests...');
  runWithInheritedOutput('cargo', ['test', '--workspace', '--exclude', 'astrcode']);
}

if (stagedFrontendFiles.length > 0) {
  // Reuse the repository's local Prettier install instead of assuming a global formatter is present.
  console.log(`pre-commit: formatting ${stagedFrontendFiles.length} frontend file(s).`);
  runWithInheritedOutput(npxCommand, ['--prefix', 'frontend', 'prettier', '--write', ...stagedFrontendFiles]);
}

// Re-stage formatter output so the commit always contains the exact code that was auto-formatted.
runWithInheritedOutput('git', ['add', '--', ...filesToFormat]);
