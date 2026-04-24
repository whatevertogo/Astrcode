# Repository Guidelines

本项目不维护向后兼容，优先良好架构与干净代码。

## 环境要求

- Rust **nightly** 工具链（见 `rust-toolchain.toml`）
- Node.js 20+
- 首次安装：`npm install && cd frontend && npm install`

## 常用命令

```bash
# 开发
npm run dev:tauri             # Tauri 桌面端开发
cargo run -p astrcode-server  # 只启动后端
cd frontend && npm run dev    # 只启动前端
cargo run -p astrcode-cli     # 终端 TUI

# 构建与检查
npm run build                # Tauri 桌面端构建
cargo check --workspace      # 快速编译检查
cargo test --workspace --exclude astrcode --lib  # push 前快速测试

# 完整 CI 检查
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
node scripts/check-crate-boundaries.mjs
cd frontend && npm run typecheck && npm run lint && npm run format:check

# 架构守卫
node scripts/check-crate-boundaries.mjs --strict  # 严格模式
node scripts/generate-crate-deps-graph.mjs --check # 依赖图同步检查

# 真实 API eval
npm run eval:api -- --task-set eval-tasks/task-set.yaml --concurrency 1
```

## 架构约束

详见 `PROJECT_ARCHITECTURE.md`，以下为摘要：

- `server` 是唯一组合根，通过 `bootstrap_server_runtime()` 组装所有组件
- `host-session` 持有 session durable truth；`agent-runtime` 只负责单 turn 执行；`context-window` 负责 compact 和请求整形
- `plugin-host` 持有 plugin/MCP/builtin 贡献的 active snapshot
- adapter 之间禁止横向依赖；共享边界必须进入 contract crate 或 owner port
- 能力语义统一使用 `CapabilitySpec`（`astrcode-core`），传输层使用 `CapabilityWireDescriptor`（`astrcode-protocol`）

## Rust 命名与设计要求

- 命名必须清晰、直接、可预测，优先让人一眼看懂语义。
- 类型、Trait、枚举变体使用 `UpperCamelCase`。
- 模块、函数、方法、变量使用 `snake_case`。
- 常量使用 `SCREAMING_SNAKE_CASE`。
- 类型名应为名词，函数名应为动作，布尔变量名应表达判断语义，如 `is_*`、`has_*`、`can_*`。
- 禁止使用含义模糊的命名，如 `manager`、`helper`、`util`、`common`、`base`，除非语义确实准确且不可替代。

## 设计原则

- 遵循单一职责：一个模块、类型、Trait、函数只负责一类清晰职责。
- 遵循关注点分离：领域模型、业务编排、存储/网络/文件等副作用、协议 DTO 必须分层清晰，避免混杂。
- 优先用类型表达语义：能用 `enum`、新类型、结构体表达的，不要依赖裸 `String`、裸 `u64`、魔法 `bool`。
- 上层依赖抽象而非具体实现；优先依赖 Trait，而不是直接耦合底层实现。
- 公共接口保持最小且稳定，非必要不暴露内部细节。

## 编码约束

- 函数应短小、直接，参数语义必须明确。
- 参数过多或存在多种可选配置时，优先使用 Builder，禁止堆叠位置参数。
- 禁止用布尔参数表达模式分支，优先改为具名枚举。
- 能显式表达语义时，不要引入隐式行为或过度魔法。
- 优先可读性，避免炫技、过度抽象和无必要设计模式。

## 自检标准

提交前确认：
- 是否能从命名直接理解职责？
- 是否一个类型/函数只做一件主要事情？
- 是否副作用与核心逻辑已分离？
- 是否减少了调用方的理解成本？
- 是否让未来修改更容易而不是更困难？

## Gotchas

- 文档必须使用中文
- 使用 `node scripts/check-crate-boundaries.mjs` 验证 crate 依赖规则没有被违反
- `src-tauri` 是 Tauri 薄壳，不含业务逻辑
- `server` 组合根在 `crates/server/src/bootstrap/runtime.rs`
- CI 只跑 eval smoke；真实模型评测走 `npm run eval:api`
