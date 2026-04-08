import { run } from './hook-utils.mjs';

// 阶段5升级：默认开启强阻断模式，避免越界依赖被静默忽略。
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
      description: 'protocol 必须保持纯 DTO，不得依赖 core/runtime 系列',
      source: 'astrcode-protocol',
      forbidden: [/^astrcode-core$/, /^astrcode-runtime(?:-.+)?$/],
    },
    {
      id: 'R002',
      description: 'runtime-prompt 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-prompt',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-prompt']),
    },
    {
      id: 'R003',
      description: 'runtime-llm 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-llm',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-llm']),
    },
    {
      id: 'R004',
      description: 'runtime-config 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-config',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-config']),
    },
    // --- 阶段4结构性解耦新增规则 ---
    {
      id: 'R005',
      description: 'runtime-execution 不得直接依赖 runtime-skill-loader',
      source: 'astrcode-runtime-execution',
      forbidden: [/^astrcode-runtime-skill-loader$/],
    },
    {
      id: 'R006',
      description: 'runtime-execution 不得直接依赖 runtime-agent-loop',
      source: 'astrcode-runtime-execution',
      forbidden: [/^astrcode-runtime-agent-loop$/],
    },
    {
      id: 'R007',
      description: 'runtime-execution 不得直接依赖 runtime-agent-tool',
      source: 'astrcode-runtime-execution',
      forbidden: [/^astrcode-runtime-agent-tool$/],
    },
    // --- runtime 系列其他编译隔离 ---
    {
      id: 'R008',
      description: 'runtime-session 编译隔离：不得直接依赖其他 runtime-* crate（除 core/agent-control/agent-loop）',
      source: 'astrcode-runtime-session',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set([
        'astrcode-runtime-session',
        'astrcode-runtime-agent-control',
        'astrcode-runtime-agent-loop',
      ]),
    },
    {
      id: 'R009',
      description: 'runtime-agent-control 编译隔离：不得直接依赖其他 runtime-* crate（除 core/config）',
      source: 'astrcode-runtime-agent-control',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set([
        'astrcode-runtime-agent-control',
        'astrcode-runtime-config',
      ]),
    },
    {
      id: 'R010',
      description: 'runtime-registry 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-registry',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-registry']),
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

    if (rule.forbidden) {
      const blocked = rule.forbidden.some((pattern) => pattern.test(dep));
      const allowedByException = rule.allowForbiddenExact?.has(dep) ?? false;
      if (blocked && !allowedByException) {
        violations.push(dep);
      }
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
    if (!packageNames.has(rule.source)) {
      continue;
    }
    const violations = checkRule(rule, edges, packageNames);
    if (violations.length > 0) {
      findings.push({ rule, violations: violations.sort() });
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
