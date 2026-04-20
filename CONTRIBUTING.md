# 贡献指南

感谢你愿意参与 AstrCode。

## 开始前

- 先通读 [README.md](README.md) 与 [PROJECT_ARCHITECTURE.md](PROJECT_ARCHITECTURE.md)
- 使用仓库要求的 Rust `nightly` 与 Node.js 20+
- 首次进入仓库后执行：

```bash
npm install
cd frontend && npm install
```

## 开发方式

常用命令：

```bash
# 桌面端开发
cargo tauri dev

# 仅后端
cargo run -p astrcode-server

# 仅前端
cd frontend && npm run dev

# CLI
cargo run -p astrcode-cli
```

## 提交前检查

请至少运行与你改动直接相关的检查；提交前默认建议跑这一组：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
node scripts/check-crate-boundaries.mjs
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

## 代码约定

- 文档与注释使用中文
- 优先根治问题，不做表面补丁
- 命名要直接表达语义
- 不维护向后兼容，优先干净架构
- `src-tauri` 仅作为 Tauri 薄壳，不承载业务逻辑
- `server` 是组合根，跨层依赖必须遵守 [PROJECT_ARCHITECTURE.md](PROJECT_ARCHITECTURE.md)

## Pull Request 期望

- 描述清楚动机、行为变化和验证方式
- 尽量保持单一主题，不把无关重构混入同一个 PR
- 涉及架构边界、协议、权限模型或核心依赖时，请明确说明取舍
- UI 改动尽量附截图或录屏

## Issue 与沟通

- Bug 与功能建议：使用 GitHub Issue 模板
- 安全问题：不要公开提 issue，按 [SECURITY.md](SECURITY.md) 里的方式报告
- 不确定需求方向时，先开 issue 或 draft PR 讨论
