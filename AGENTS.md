# Repository Guidelines

本项目不维护向后兼容，优先良好架构与干净代码。

## 环境要求

- Rust **nightly** 工具链（见 `rust-toolchain.toml`）
- Node.js 20+
- 首次安装：`npm install && cd frontend && npm install`

## 常用命令

```bash
# 开发
cargo tauri dev             # Tauri 桌面端开发
cargo run -p astrcode-server  # 只启动后端
cd frontend && npm run dev    # 只启动前端

# 构建与检查
cargo tauri build            # Tauri 桌面端构建
cargo check --workspace      # 快速编译检查
cargo test --workspace --exclude astrcode --lib  # push 前快速测试

# 完整 CI 检查
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
node scripts/check-crate-boundaries.mjs
cd frontend && npm run typecheck && npm run lint && npm run format:check

# 架构守卫
node scripts/check-crate-boundaries.mjs          # 检查 crate 依赖边界
node scripts/check-crate-boundaries.mjs --strict  # 严格模式
```

## 架构约束

详见 `PROJECT_ARCHITECTURE.md`，以下为摘要：

- `server` 是唯一组合根，通过 `bootstrap_server_runtime()` 组装所有组件
- `application` 不依赖任何 `adapter-*`，只依赖 `core` + `kernel` + `session-runtime`
- 治理层使用 `AppGovernance`（`astrcode-application`）
- 能力语义统一使用 `CapabilitySpec`（`astrcode-core`），传输层使用 `CapabilityWireDescriptor`（`astrcode-protocol`）

## 代码规范

- 用中文注释，且注释尽量表明为什么和做了什么
- 不需要向后兼容，优先良好架构,期望最佳实践而不是打补丁
- Git 提交信息使用 emoji + type + scope 风格（如 `✨ feat(module): brief description`）

## 提交前验证

改了后端rust代码每次提交前按顺序执行：

1. `cargo fmt --all` — 格式化代码
2. `cargo clippy --all-targets --all-features -- -D warnings` — 修复所有警告
3. `cargo test --workspace` — 确保所有测试通过
4. 确认变更内容后写出描述性提交信息

改了前端代码每次提交前按顺序执行：
1. `npm run format` — 格式化代码
2. `npm run lint` — 修复所有 lint 错误
3. `npm run typecheck` — 确保没有类型错误
4. `npm run format:check` — 确保格式正确

## Gotchas

- 前端css不允许出现webview相关内容这会导致应用端无法下滑窗口
- 文档必须使用中文
- 使用 `node scripts/check-crate-boundaries.mjs` 验证 crate 依赖规则没有被违反
- `src-tauri` 是 Tauri 薄壳，不含业务逻辑
- `server` 组合根在 `crates/server/src/bootstrap/runtime.rs`

## TUI style conventions

See `codex-rs/tui/styles.md`.

## TUI code conventions

- Use concise styling helpers from ratatui’s Stylize trait.
  - Basic spans: use "text".into()
  - Styled spans: use "text".red(), "text".green(), "text".magenta(), "text".dim(), etc.
  - Prefer these over constructing styles with `Span::styled` and `Style` directly.
  - Example: patch summary file lines
    - Desired: vec!["  └ ".into(), "M".red(), " ".dim(), "tui/src/app.rs".dim()]

### TUI Styling (ratatui)

- Prefer Stylize helpers: use "text".dim(), .bold(), .cyan(), .italic(), .underlined() instead of manual Style where possible.
- Prefer simple conversions: use "text".into() for spans and vec![…].into() for lines; when inference is ambiguous (e.g., Paragraph::new/Cell::from), use Line::from(spans) or Span::from(text).
- Computed styles: if the Style is computed at runtime, using `Span::styled` is OK (`Span::from(text).set_style(style)` is also acceptable).
- Avoid hardcoded white: do not use `.white()`; prefer the default foreground (no color).
- Chaining: combine helpers by chaining for readability (e.g., url.cyan().underlined()).
- Single items: prefer "text".into(); use Line::from(text) or Span::from(text) only when the target type isn’t obvious from context, or when using .into() would require extra type annotations.
- Building lines: use vec![…].into() to construct a Line when the target type is obvious and no extra type annotations are needed; otherwise use Line::from(vec![…]).
- Avoid churn: don’t refactor between equivalent forms (Span::styled ↔ set_style, Line::from ↔ .into()) without a clear readability or functional gain; follow file‑local conventions and do not introduce type annotations solely to satisfy .into().
- Compactness: prefer the form that stays on one line after rustfmt; if only one of Line::from(vec![…]) or vec![…].into() avoids wrapping, choose that. If both wrap, pick the one with fewer wrapped lines.

### Text wrapping

- Always use textwrap::wrap to wrap plain strings.
- If you have a ratatui Line and you want to wrap it, use the helpers in tui/src/wrapping.rs, e.g. word_wrap_lines / word_wrap_line.
- If you need to indent wrapped lines, use the initial_indent / subsequent_indent options from RtOptions if you can, rather than writing custom logic.
- If you have a list of lines and you need to prefix them all with some prefix (optionally different on the first vs subsequent lines), use the `prefix_lines` helper from line_utils.