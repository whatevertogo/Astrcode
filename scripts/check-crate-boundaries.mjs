import { run } from './hook-utils.mjs';

const STRICT_MODE = process.argv.includes('--strict');

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
      description: 'runtime-tool-loader 仅允许依赖 core（内部 crate）',
      source: 'astrcode-runtime-tool-loader',
      allowedExact: new Set(['astrcode-core']),
    },
    {
      id: 'R003',
      description: 'runtime-prompt 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-prompt',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-prompt']),
    },
    {
      id: 'R004',
      description: 'runtime-llm 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-llm',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-llm']),
    },
    {
      id: 'R005',
      description: 'runtime-config 编译隔离：不得直接依赖其他 runtime-* crate',
      source: 'astrcode-runtime-config',
      forbidden: [/^astrcode-runtime(?:-.+)?$/],
      allowForbiddenExact: new Set(['astrcode-runtime-config']),
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
      if (!STRICT_MODE) {
        console.log(`::warning title=crate-boundary::${finding.rule.source} should not depend on ${dep} (${finding.rule.id})`);
      }
    }
  }

  if (STRICT_MODE) {
    process.exit(1);
  }

  console.log('soft mode enabled: violations reported as warnings only.');
}

main();
