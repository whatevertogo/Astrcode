import {
  listFilesWithUnstagedChanges,
  listStagedFiles,
  readStagedBlob,
  repoRoot,
  runWithInheritedOutput,
  stagedBlobSize,
  textDecoder,
  unique,
  isProbablyBinary,
} from './hook-utils.mjs';

const MAX_STAGED_FILE_BYTES = 1024 * 1024;
const CONFLICT_MARKER_PATTERN = /^(<<<<<<< .+|=======|>>>>>>> .+|\|\|\|\|\|\|\| .+)$/mu;
const SECRET_PATTERNS = [
  { label: 'private key', pattern: /-----BEGIN [A-Z ]*PRIVATE KEY-----/u },
  { label: 'GitHub token', pattern: /\bgh[pousr]_[A-Za-z0-9_]{20,}\b/u },
  { label: 'OpenAI-style secret', pattern: /\bsk-[A-Za-z0-9]{20,}\b/u },
  { label: 'AWS access key', pattern: /\bAKIA[0-9A-Z]{16}\b/u },
  { label: 'Google API key', pattern: /\bAIza[0-9A-Za-z\-_]{35}\b/u },
  { label: 'Slack token', pattern: /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/u },
];

const stagedFiles = listStagedFiles();
if (stagedFiles.length === 0) {
  console.log('pre-commit: no staged files to inspect.');
  process.exit(0);
}

// Auto-generate crate dependency graph if Cargo files or crate source changed
const hasCargoChanges = stagedFiles.some(
  (file) =>
    file === 'Cargo.toml' ||
    file === 'Cargo.lock' ||
    (file.startsWith('crates/') && (file.endsWith('Cargo.toml') || file.endsWith('.rs'))),
);

if (hasCargoChanges) {
  console.log('pre-commit: detected Cargo changes, regenerating dependency graph.');
  runWithInheritedOutput('node', ['scripts/generate-crate-deps-graph.mjs'], { cwd: repoRoot });
  runWithInheritedOutput('git', ['add', 'docs/architecture/crates-dependency-graph.md'], { cwd: repoRoot });
}

const stagedRustFiles = stagedFiles.filter((file) => file.endsWith('.rs'));
const stagedFrontendFormatFiles = stagedFiles.filter((file) =>
  /^frontend\/src\/.+\.(ts|tsx|css)$/u.test(file),
);
const stagedFrontendLintFiles = stagedFiles.filter((file) =>
  /^frontend\/src\/.+\.(ts|tsx)$/u.test(file),
);
const filesToRestage = unique([...stagedRustFiles, ...stagedFrontendFormatFiles]);
const filesWithUnstagedChanges = listFilesWithUnstagedChanges(filesToRestage);

if (filesWithUnstagedChanges.length > 0) {
  // Auto-fix hooks must refuse mixed staged/unstaged hunks, otherwise they silently rewrite code
  // the author did not intend to include in this commit.
  console.error(
    `pre-commit: refusing to auto-fix files with unstaged changes: ${filesWithUnstagedChanges.join(', ')}`,
  );
  console.error('pre-commit: stage or stash those edits first so the hook only rewrites the visible patch.');
  process.exit(1);
}

if (stagedRustFiles.length > 0) {
  // Keep Rust formatting in pre-commit because it is deterministic, cheap, and fixes most style
  // drift without forcing the author to rerun a slower validation suite.
  console.log(`pre-commit: formatting ${stagedRustFiles.length} Rust file(s).`);
  runWithInheritedOutput('cargo', ['fmt', '--all', '--', ...stagedRustFiles], { cwd: repoRoot });
}

if (stagedFrontendFormatFiles.length > 0) {
  // Prettier is the fastest way to normalize TS/TSX/CSS diffs before review, so it belongs in the
  // zero-friction commit path.
  console.log(`pre-commit: formatting ${stagedFrontendFormatFiles.length} frontend file(s).`);
  runWithInheritedOutput('npx', ['--prefix', 'frontend', 'prettier', '--write', ...stagedFrontendFormatFiles], {
    cwd: repoRoot,
  });
}

if (stagedFrontendLintFiles.length > 0) {
  // Restrict ESLint to changed source files so pre-commit catches obvious local issues without
  // turning every commit into a repo-wide lint pass.
  // ESLint 的 flat config 位于 frontend/eslint.config.js，必须从前端目录运行才能正确加载配置，
  // 因此去掉路径前端的 'frontend/' 前缀，改为从前端目录执行。
  console.log(`pre-commit: lint-fixing ${stagedFrontendLintFiles.length} frontend TS/TSX file(s).`);
  const relativeFiles = stagedFrontendLintFiles.map((f) => f.replace(/^frontend\//u, ''));
  runWithInheritedOutput('npx', ['eslint', '--fix', ...relativeFiles], {
    cwd: `${repoRoot}/frontend`,
  });
}

if (filesToRestage.length > 0) {
  runWithInheritedOutput('git', ['add', '--', ...filesToRestage], { cwd: repoRoot });
}

const failures = [];
for (const file of stagedFiles) {
  const size = stagedBlobSize(file);
  if (size > MAX_STAGED_FILE_BYTES) {
    failures.push(`${file}: staged blob is ${size} bytes, which exceeds the 1 MiB pre-commit limit`);
    continue;
  }

  const blob = readStagedBlob(file);
  if (isProbablyBinary(blob)) {
    continue;
  }

  const content = textDecoder.decode(blob);
  if (CONFLICT_MARKER_PATTERN.test(content)) {
    failures.push(`${file}: contains unresolved merge conflict markers`);
  }

  for (const { label, pattern } of SECRET_PATTERNS) {
    if (pattern.test(content)) {
      failures.push(`${file}: looks like it contains a ${label}`);
    }
  }
}

if (failures.length > 0) {
  console.error('pre-commit: quick static guards failed:');
  for (const failure of failures) {
    console.error(`  - ${failure}`);
  }
  process.exit(1);
}

console.log('pre-commit: fast checks passed.');
