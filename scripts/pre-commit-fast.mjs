import {
  listFilesWithUnstagedChanges,
  listStagedFiles,
  repoRoot,
  runWithInheritedOutput,
  unique,
} from './hook-utils.mjs';

import { readStagedBlob, isProbablyBinary, textDecoder } from './hook-utils.mjs';

const stagedFiles = listStagedFiles();
if (stagedFiles.length === 0) {
  console.log('pre-commit: no staged files to inspect.');
  process.exit(0);
}

// ── 安全守卫：冲突标记 & 常见密钥模式 ──────────────────────────
const CONFLICT_RE = /^(<{7}|={7}|>{7})/mu;
const SECRET_RE =
  /(?:(?:AKIA|ABIA|ACCA|ASIA)[0-9A-Z]{16}|-----BEGIN (?:RSA |EC )?PRIVATE KEY-----|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{36,}|sk_live_[0-9a-zA-Z]{24,})/u;

let safetyBlocked = false;
for (const file of stagedFiles) {
  let blob;
  try {
    blob = readStagedBlob(file);
  } catch {
    // 新增的空文件或已删除文件，跳过
    continue;
  }
  if (isProbablyBinary(blob)) continue;

  const content = textDecoder.decode(blob);
  if (CONFLICT_RE.test(content)) {
    console.error(`pre-commit: ${file} contains unresolved merge conflict markers.`);
    safetyBlocked = true;
  }
  if (SECRET_RE.test(content)) {
    console.error(`pre-commit: ${file} may contain a secret (API key / private key).`);
    safetyBlocked = true;
  }
}
if (safetyBlocked) {
  console.error('pre-commit: resolve the above issues before committing.');
  process.exit(1);
}

// Cargo 文件或 crate 源码变更时自动重新生成依赖图
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
const filesToRestage = unique([...stagedRustFiles, ...stagedFrontendFormatFiles]);
const filesWithUnstagedChanges = listFilesWithUnstagedChanges(filesToRestage);

if (filesWithUnstagedChanges.length > 0) {
  // 自动格式化只能作用于全部已暂存的文件，混合暂存/未暂存修改会导致静默重写作者未意图包含的代码。
  console.error(
    `pre-commit: refusing to auto-fix files with unstaged changes: ${filesWithUnstagedChanges.join(', ')}`,
  );
  console.error('pre-commit: stage or stash those edits first so the hook only rewrites the visible patch.');
  process.exit(1);
}

if (stagedRustFiles.length > 0) {
  // Rust 格式化是确定性的、快速的，能在提交前修正大部分风格偏差。
  console.log(`pre-commit: formatting ${stagedRustFiles.length} Rust file(s).`);
  runWithInheritedOutput('cargo', ['fmt', '--all', '--', ...stagedRustFiles], { cwd: repoRoot });
}

if (stagedFrontendFormatFiles.length > 0) {
  // Prettier 能快速规范化 TS/TSX/CSS 差异，适合零摩擦提交路径。
  console.log(`pre-commit: formatting ${stagedFrontendFormatFiles.length} frontend file(s).`);
  runWithInheritedOutput('npx', ['--prefix', 'frontend', 'prettier', '--write', ...stagedFrontendFormatFiles], {
    cwd: repoRoot,
  });
}

if (filesToRestage.length > 0) {
  runWithInheritedOutput('git', ['add', '--', ...filesToRestage], { cwd: repoRoot });
}

console.log('pre-commit: format checks passed.');
