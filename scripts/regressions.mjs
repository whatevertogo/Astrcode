import { repoRoot, runWithInheritedOutput } from './hook-utils.mjs';

// 阶段 0 基线回归套件：覆盖当前架构下的关键链路。
const checks = [
  {
    name: 'prompt build cache regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-adapter-prompt',
      '--lib',
      'layered_builder::tests::inherited_cache_reuses_compact_summary_without_reusing_recent_tail',
    ],
  },
  {
    name: 'agent loop basic lifecycle regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-agent-runtime',
      '--lib',
      'r#loop::tests::execute_empty_turn_emits_basic_lifecycle',
    ],
  },
  {
    name: 'tool dispatch round-trip regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-agent-runtime',
      '--lib',
      'r#loop::tests::tool_dispatch_results_continue_back_to_provider',
    ],
  },
];

for (const check of checks) {
  console.log(`phase0 regression: running ${check.name}`);
  runWithInheritedOutput(check.command, check.args, { cwd: repoRoot });
}

console.log('phase0 regression suite passed.');
