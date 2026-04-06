import { repoRoot, runWithInheritedOutput } from './hook-utils.mjs';

// 阶段 0 基线回归套件：覆盖会话执行、插件能力、prompt 构建三条关键链路。
const checks = [
  {
    name: 'session execution regression',
    command: 'cargo',
    args: ['test', '-p', 'astrcode-runtime', '--lib', 'service::execution::tests'],
  },
  {
    name: 'plugin capability regression',
    command: 'cargo',
    args: ['test', '-p', 'astrcode-plugin', '--test', 'v4_stdio_e2e'],
  },
  {
    name: 'prompt build regression',
    command: 'cargo',
    args: ['test', '-p', 'astrcode-runtime-prompt', '--lib'],
  },
];

for (const check of checks) {
  console.log(`phase0 regression: running ${check.name}`);
  runWithInheritedOutput(check.command, check.args, { cwd: repoRoot });
}

console.log('phase0 regression suite passed.');
