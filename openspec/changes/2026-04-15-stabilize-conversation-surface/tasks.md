## 1. Conversation 命名收口

- [x] 1.1 在 `crates/client/`、`crates/cli/` 内部实现中统一以 `conversation` 作为主命名，`terminal` 仅保留薄兼容 alias。
- [x] 1.2 更新验收脚本与相关说明文案，统一描述为 conversation contract / terminal frontend acceptance。

## 2. CLI 状态与控制器硬化

- [x] 2.1 将 `CliState` 稳定为 `shell / conversation / interaction / render / stream` 五个子域聚合。
- [x] 2.2 继续收口 `AppController`，把 hydrate、stream switch、slash refresh 等会话协调逻辑从主 action 分发中拆出。
- [x] 2.3 补强 CLI 单测，覆盖 snapshot 激活、overlay 与 composer 隔离、stream batch 切换和 resize cache invalidation。

## 3. Session Runtime 边界硬化

- [x] 3.1 在 `session-runtime` 内继续把读写入口分别沉到 `query` 与 `command` 子域，由 `SessionRuntime` 只做薄委托。
- [x] 3.2 收紧 `application` 侧 session 端口，减少对 `SessionState` 与原始 stored event access 的直接暴露。
- [x] 3.3 用测试冻结 mailbox、observe、turn terminal projection 等既有行为，确保结构调整不改语义。

## 4. 文档与 Specs 对齐

- [x] 4.1 更新 `PROJECT_ARCHITECTURE.md`，明确 `SessionState = projection cache + live execution control`，并区分 `TurnState` 与 `SessionPhase`。
- [x] 4.2 更新相关 specs，统一“conversation surface consumed by terminal frontend”的描述。
