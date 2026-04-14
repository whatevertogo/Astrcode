# Repository Guidelines

本项目不维护向后兼容，优先良好架构与干净代码。

## 环境要求

- Rust **nightly** 工具链（见 `rust-toolchain.toml`）
- Node.js 20+
- 首次安装：`npm install && cd frontend && npm install`

## 常用命令

```bash
# 开发
make dev              # Windows 桌面端开发
make dev-unix         # Linux/macOS 桌面端开发
make frontend         # 只启动前端
cargo run -p astrcode-server  # 只启动后端

# 构建与检查
make build            # Tauri 桌面端构建
make check            # push 前快速检查
make check-ci         # 与 CI 对齐的完整检查

# 架构守卫
make check-boundaries        # 检查 crate 依赖边界
make check-boundaries-strict # 严格模式

# 前端
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

## 架构约束

详见 `PROJECT_ARCHITECTURE.md`，以下为摘要：

- `server` 是唯一组合根，通过 `bootstrap_server_runtime()` 组装所有组件
- `application` 不依赖任何 `adapter-*`，只依赖 `core` + `kernel` + `session-runtime`
- 治理层使用 `AppGovernance`（`astrcode-application`），不使用旧 `RuntimeGovernance`（`astrcode-runtime`）
- 能力语义统一使用 `CapabilitySpec`（`astrcode-core`），传输层使用 `CapabilityDescriptor`（`astrcode-protocol`）

## 代码规范

- 用中文注释，且注释尽量表明为什么和做了什么
- 不需要向后兼容，优先良好架构,期望最佳实践而不是打补丁
- Git 提交信息使用 emoji + type + scope 风格（如 `✨ feat(module): brief description`）

## 提交前验证

每次提交前按顺序执行：

1. `cargo fmt --all` — 格式化代码
2. `cargo clippy --all-targets --all-features -- -D warnings` — 修复所有警告
3. `cargo test --workspace` — 确保所有测试通过
4. 确认变更内容后写出描述性提交信息

## Gotchas

- 前端css不允许出现webview相关内容这会导致应用端无法下滑窗口
- 文档必须使用中文
- 使用 `make check-boundaries` 验证 crate 依赖规则没有被违反
- `src-tauri` 是 Tauri 薄壳，不含业务逻辑
- `server` 组合根在 `crates/server/src/bootstrap/runtime.rs`

## 变更管理

使用 OpenSpec 管理变更：

- 新变更：`openspec/changes/<change-name>/` 目录
- 归档前需合并 delta specs 到 `openspec/specs/` 主目录
- 已归档变更存放在 `openspec/archive/`
