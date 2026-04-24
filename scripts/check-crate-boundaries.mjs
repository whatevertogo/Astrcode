import { run } from './hook-utils.mjs';

// 默认开启强阻断模式，避免架构越界依赖被静默忽略。
// 若需临时降级为警告模式，可传 --soft 参数。
const SOFT_MODE = process.argv.includes('--soft');

function parseMetadata() {
  const raw = run('cargo', ['metadata', '--format-version', '1', '--no-deps']);
  return JSON.parse(raw);
}

function normalizePath(input) {
  return input.replaceAll('\\', '/');
}

function getWorkspaceCrates(metadata) {
  const members = new Set(metadata.workspace_members);
  const packages = metadata.packages.filter((pkg) => members.has(pkg.id));
  const packageNames = new Set(packages.map((pkg) => pkg.name));
  const packageByManifestDir = new Map(
    packages.map((pkg) => [normalizePath(pkg.manifest_path).replace(/\/Cargo\.toml$/u, ''), pkg.name]),
  );

  const edges = new Map();
  for (const pkg of packages) {
    edges.set(pkg.name, new Set());
  }

  for (const source of packages) {
    for (const dep of source.dependencies ?? []) {
      const depPath = dep.path ? normalizePath(dep.path) : null;
      const targetByPath = depPath ? packageByManifestDir.get(depPath) : null;
      const target = targetByPath ?? dep.name;
      if (packageNames.has(target)) {
        edges.get(source.name)?.add(target);
      }
    }
  }

  return { packages, edges };
}

function buildRules() {
  return [
    {
      id: 'R001',
      description: 'core 是领域根，不得依赖任何其他工作区 crate',
      source: 'astrcode-core',
      allowedExact: new Set(),
    },
    {
      id: 'R002',
      description: 'protocol 必须保持纯 DTO，仅允许依赖 core 与对外传输所需的 contract crate',
      source: 'astrcode-protocol',
      allowedExact: new Set(['astrcode-core', 'astrcode-governance-contract']),
    },
    {
      id: 'R003',
      description: 'prompt-contract 只承载 prompt 契约，仅允许依赖 core',
      source: 'astrcode-prompt-contract',
      allowedExact: new Set(['astrcode-core']),
    },
    {
      id: 'R004',
      description: 'governance-contract 只承载治理契约，仅允许依赖 core、prompt-contract',
      source: 'astrcode-governance-contract',
      allowedExact: new Set(['astrcode-core', 'astrcode-prompt-contract']),
    },
    {
      id: 'R005',
      description: 'tool-contract 只承载工具契约，仅允许依赖 core、governance-contract',
      source: 'astrcode-tool-contract',
      allowedExact: new Set(['astrcode-core', 'astrcode-governance-contract']),
    },
    {
      id: 'R006',
      description: 'support 仅允许依赖 core',
      source: 'astrcode-support',
      allowedExact: new Set(['astrcode-core']),
    },
    {
      id: 'R007',
      description: 'llm-contract 只承载 LLM 契约，仅允许依赖 core、governance-contract、prompt-contract',
      source: 'astrcode-llm-contract',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-governance-contract',
        'astrcode-prompt-contract',
      ]),
    },
    {
      id: 'R008',
      description: 'runtime-contract 只承载 runtime 边界，仅允许依赖 core、llm-contract、tool-contract',
      source: 'astrcode-runtime-contract',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-llm-contract',
        'astrcode-tool-contract',
      ]),
    },
    {
      id: 'R009',
      description: 'context-window 只负责上下文窗口，允许依赖 core、llm-contract、runtime-contract、tool-contract、support',
      source: 'astrcode-context-window',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-llm-contract',
        'astrcode-runtime-contract',
        'astrcode-tool-contract',
        'astrcode-support',
      ]),
    },
    {
      id: 'R010',
      description: 'agent-runtime 是最小执行内核，仅允许依赖 core、context-window、llm-contract、runtime-contract、tool-contract',
      source: 'astrcode-agent-runtime',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-context-window',
        'astrcode-llm-contract',
        'astrcode-prompt-contract',
        'astrcode-runtime-contract',
        'astrcode-tool-contract',
      ]),
    },
    {
      id: 'R011',
      description: 'plugin-host 只承载统一插件宿主，只允许依赖 core、protocol、governance-contract、support',
      source: 'astrcode-plugin-host',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-governance-contract',
        'astrcode-protocol',
        'astrcode-support',
      ]),
    },
    {
      id: 'R012',
      description: 'host-session 只承载 session owner 逻辑，只允许依赖 core、support、agent-runtime、plugin-host、governance-contract、prompt-contract、runtime-contract、tool-contract',
      source: 'astrcode-host-session',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-support',
        'astrcode-agent-runtime',
        'astrcode-plugin-host',
        'astrcode-governance-contract',
        'astrcode-prompt-contract',
        'astrcode-runtime-contract',
        'astrcode-tool-contract',
      ]),
    },
  ];
}

function isWorkspaceInternal(crateName, packageNames) {
  return packageNames.has(crateName);
}

function checkRule(rule, edges, packageNames) {
  const deps = [...(edges.get(rule.source) ?? [])].filter((name) =>
    isWorkspaceInternal(name, packageNames),
  );

  const violations = [];
  for (const dep of deps) {
    if (rule.allowedExact && !rule.allowedExact.has(dep)) {
      violations.push(dep);
      continue;
    }
  }

  return violations;
}

function main() {
  const metadata = parseMetadata();
  const { packages, edges } = getWorkspaceCrates(metadata);
  const packageNames = new Set(packages.map((item) => item.name));
  const rules = buildRules();
  const findings = [];

  for (const rule of rules) {
    const violations = checkRule(rule, edges, packageNames);
    if (violations.length > 0) {
      findings.push({ rule, violations: violations.sort() });
    }
  }

  for (const source of packageNames) {
    if (!source.startsWith('astrcode-adapter-')) {
      continue;
    }
    const deps = [...(edges.get(source) ?? [])].filter((name) => isWorkspaceInternal(name, packageNames));
    const violations = deps.filter((target) =>
      target.startsWith('astrcode-adapter-') && target !== 'astrcode-adapter-storage',
    );
    if (violations.length > 0) {
      findings.push({
        rule: {
          id: 'R013',
          description: 'adapter-* 之间禁止横向依赖',
          source,
        },
        violations: violations.sort(),
      });
    }
  }

  if (findings.length === 0) {
    console.log('crate boundary check passed.');
    return;
  }

  console.log('crate boundary check found violations:');
  for (const finding of findings) {
    console.log(`- ${finding.rule.id} ${finding.rule.description}`);
    for (const dep of finding.violations) {
      console.log(`  - forbidden dependency: ${finding.rule.source} -> ${dep}`);
      if (SOFT_MODE) {
        console.log(`::warning title=crate-boundary::${finding.rule.source} should not depend on ${dep} (${finding.rule.id})`);
      }
    }
  }

  if (!SOFT_MODE) {
    process.exit(1);
  }

  console.log('::warning title=crate-boundary::Violations found but running in soft mode. Add --soft to downgrade to warnings only.');
  console.log('soft mode enabled: violations reported as warnings only.');
}

main();
