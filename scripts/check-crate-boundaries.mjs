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
      description: 'protocol 必须保持纯 DTO，仅允许依赖 core',
      source: 'astrcode-protocol',
      allowedExact: new Set(['astrcode-core']),
    },
    {
      id: 'R003',
      description: 'kernel 仅承载全局控制面，只允许依赖 core',
      source: 'astrcode-kernel',
      allowedExact: new Set(['astrcode-core']),
    },
    {
      id: 'R004',
      description: 'session-runtime 仅允许依赖 core 与 kernel',
      source: 'astrcode-session-runtime',
      allowedExact: new Set(['astrcode-core', 'astrcode-kernel']),
    },
    {
      id: 'R005',
      description: 'application 仅允许依赖 core、kernel、session-runtime',
      source: 'astrcode-application',
      allowedExact: new Set([
        'astrcode-core',
        'astrcode-kernel',
        'astrcode-session-runtime',
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
