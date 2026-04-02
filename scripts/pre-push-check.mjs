import { resolve } from 'node:path';

import { repoRoot, runWithInheritedOutput } from './hook-utils.mjs';

console.log('pre-push: running cargo check --workspace');
runWithInheritedOutput('cargo', ['check', '--workspace'], { cwd: repoRoot });

// Pre-push should stay meaningfully lighter than CI, so it runs the unit-test-heavy lib target
// subset instead of the full workspace test matrix.
console.log('pre-push: running cargo test --workspace --exclude astrcode --lib');
runWithInheritedOutput('cargo', ['test', '--workspace', '--exclude', 'astrcode', '--lib'], {
  cwd: repoRoot,
});

console.log('pre-push: running frontend typecheck');
runWithInheritedOutput('npm', ['run', 'typecheck'], { cwd: resolve(repoRoot, 'frontend') });

console.log('pre-push: medium checks passed.');
