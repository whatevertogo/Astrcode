# Repository Guidelines

本项目不需要向后兼容的代码，只需要良好代码架构和干净代码

## 架构约束

- `server` 是唯一组合根，通过 `bootstrap_server_runtime()` 组装所有组件
- `application` 不依赖任何 `adapter-*`，只依赖 `core` + `kernel` + `session-runtime`
- 治理层使用 `AppGovernance`（`astrcode-application`），不使用旧 `RuntimeGovernance`（`astrcode-runtime`）
- 能力语义统一使用 `CapabilitySpec`（`astrcode-core`），传输层使用 `CapabilityDescriptor`（`astrcode-protocol`）
- 旧 `crates/runtime/` 及其子 crate 仍存在但处于过渡期，新代码不应增加对它们的依赖

## 注意

- 用中文注释，且注释尽量表明为什么和做了什么
- 为了干净架构和良好实现不需要向后兼容
- 最后需要cargo fmt --all --check  && cargo clippy --all-targets --all-features -- -D warnings && cargo test验证你的更改
- 前端css不允许出现webview相关内容这会导致应用端无法下滑窗口
- 你必须用中文写文档
- Git 提交信息使用 emoji + type + scope 风格（如 `✨ feat(module): brief description`）
- 使用 OpenSpec 管理变更，changes 归档时需先合并 delta specs 到主目录

你需要了解PROJECT_ARCHITECTURE.md中的架构设计原则和模块划分，才能更好地理解和参与项目的开发
有可以顺手优化的问题顺手优化一下