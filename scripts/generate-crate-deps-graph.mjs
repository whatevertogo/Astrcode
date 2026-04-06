import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';

import { repoRoot, run } from './hook-utils.mjs';

const OUTPUT_PATH = resolve(repoRoot, 'docs/architecture/crates-dependency-graph.md');
const CHECK_MODE = process.argv.includes('--check');

function normalizePath(input) {
  return input.replaceAll('\\', '/');
}

function collectWorkspaceCrates(metadata) {
  const members = new Set(metadata.workspace_members);
  const selected = [];

  for (const pkg of metadata.packages) {
    if (!members.has(pkg.id)) {
      continue;
    }

    const manifestPath = normalizePath(pkg.manifest_path);
    if (!manifestPath.includes('/crates/')) {
      continue;
    }

    selected.push(pkg);
  }

  selected.sort((a, b) => a.name.localeCompare(b.name));
  return selected;
}

function buildInternalEdgeMap(metadata, selectedPackages) {
  const selectedNameSet = new Set(selectedPackages.map((pkg) => pkg.name));
  const packageByManifestDir = new Map();
  const edges = new Map();

  for (const pkg of selectedPackages) {
    const manifestDir = normalizePath(dirname(pkg.manifest_path));
    packageByManifestDir.set(manifestDir, pkg.name);
    edges.set(pkg.name, new Set());
  }

  for (const source of selectedPackages) {
    for (const dep of source.dependencies ?? []) {
      const depPath = dep.path ? normalizePath(dep.path) : null;
      const targetByPath = depPath ? packageByManifestDir.get(depPath) : null;
      const target = targetByPath ?? dep.name;

      if (selectedNameSet.has(target)) {
        edges.get(source.name)?.add(target);
      }
    }
  }

  return edges;
}

function renderMarkdown(selectedPackages, edges) {
  const rows = [];

  for (const pkg of selectedPackages) {
    const manifestPath = normalizePath(pkg.manifest_path);
    const relPath = normalizePath(relative(repoRoot, dirname(manifestPath)));
    const depList = [...(edges.get(pkg.name) ?? [])].sort();
    rows.push(`| ${pkg.name} | ${relPath} | ${depList.length} | ${depList.join(', ') || '-'} |`);
  }

  const mermaidLines = ['graph TD'];
  for (const pkg of selectedPackages) {
    const source = pkg.name;
    const depList = [...(edges.get(source) ?? [])].sort();
    if (depList.length === 0) {
      mermaidLines.push(`  ${source}[${source}]`);
      continue;
    }
    for (const target of depList) {
      mermaidLines.push(`  ${source}[${source}] --> ${target}[${target}]`);
    }
  }

  return `# Crates Dependency Graph\n\n自动生成文件，请勿手工编辑。\n\n- 生成命令：\`node scripts/generate-crate-deps-graph.mjs\`\n\n## Mermaid\n\n\`\`\`mermaid\n${mermaidLines.join('\n')}\n\`\`\`\n\n## Crate 依赖表\n\n| Crate | Path | Internal Deps Count | Internal Deps |\n|---|---|---:|---|\n${rows.join('\n')}\n`;
}

function main() {
  const metadataRaw = run('cargo', ['metadata', '--format-version', '1', '--no-deps']);
  const metadata = JSON.parse(metadataRaw);
  const selectedPackages = collectWorkspaceCrates(metadata);
  const edges = buildInternalEdgeMap(metadata, selectedPackages);
  const nextContent = renderMarkdown(selectedPackages, edges);

  if (CHECK_MODE) {
    const currentContent = readFileSync(OUTPUT_PATH, 'utf8');
    if (currentContent !== nextContent) {
      console.error('crate dependency graph is out of date.');
      console.error('run: node scripts/generate-crate-deps-graph.mjs');
      process.exit(1);
    }
    console.log('crate dependency graph is up to date.');
    return;
  }

  mkdirSync(dirname(OUTPUT_PATH), { recursive: true });
  writeFileSync(OUTPUT_PATH, nextContent, 'utf8');
  console.log(`generated ${normalizePath(relative(repoRoot, OUTPUT_PATH))}`);
}

main();
