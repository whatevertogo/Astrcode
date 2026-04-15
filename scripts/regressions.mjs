import { repoRoot, runWithInheritedOutput } from './hook-utils.mjs';

// 阶段 0 基线回归套件：覆盖当前架构下的三条关键链路。
// Why: 旧脚本仍引用已删除的 astrcode-runtime / astrcode-runtime-prompt，
// 会在 CI 中直接失败；这里改为锚定现存 crate 的等价回归测试。
const checks = [
  {
    name: 'session step execution regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-session-runtime',
      '--lib',
      'turn::runner::step::tests::run_single_step_returns_cancelled_when_tool_cycle_interrupts',
    ],
  },
  {
    name: 'tool cycle live/durable regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-session-runtime',
      '--lib',
      'turn::tool_cycle::tests::invoke_single_tool_emits_structured_and_live_events_immediately',
    ],
  },
  {
    name: 'plugin capability regression',
    command: 'cargo',
    args: ['test', '-p', 'astrcode-plugin', '--test', 'v4_stdio_e2e'],
  },
  {
    name: 'prompt metrics regression',
    command: 'cargo',
    args: [
      'test',
      '-p',
      'astrcode-session-runtime',
      '--lib',
      'turn::request::tests::assemble_prompt_request_emits_prompt_metrics_for_final_prompt',
    ],
  },
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
];

for (const check of checks) {
  console.log(`phase0 regression: running ${check.name}`);
  runWithInheritedOutput(check.command, check.args, { cwd: repoRoot });
}

console.log('phase0 regression suite passed.');
