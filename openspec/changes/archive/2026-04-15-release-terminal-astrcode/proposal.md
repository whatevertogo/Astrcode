## Why

Astrcode 当前只有桌面端和浏览器端，没有一个可正式发布、可长期维护的终端版入口；这使得偏 terminal 工作流的用户无法在保留现有 server 真相与全部能力语义的前提下获得一等体验。现在启动这项变更，是为了把终端端从“临时调试 UI”升级为正式产品面，并在架构上避免再复制一套前端专属状态投影逻辑。

## What Changes

- 新增一个正式发布的终端客户端 surface，基于现有 `astrcode-server` 的 HTTP / SSE / auth 能力工作，而不是直接嵌入另一套 runtime 或绕过组合根。
- 为终端客户端补齐完整聊天工作流，覆盖 prompt 提交、thinking 展示、tool streaming、subagent / child session 展示、session 创建与恢复、会话切换，以及 `/skill`、`/compact`、`/new`、`/resume` 等 slash command 交互。
- 新增面向终端客户端的稳定 read model / projection 契约，避免终端端直接复刻当前 React reducer、局部推导规则和视图拼装细节。
- 保持 `server is the truth` 不变：桌面端、浏览器端、终端端共享同一套会话真相、能力发现、执行控制与协作执行合同。
- 增加终端版的发布与回滚策略，确保它作为新增入口逐步上线，而不是替换现有桌面 / 浏览器产品面。

## Capabilities

### New Capabilities
- `terminal-chat-surface`: 定义正式发布的终端版 Astrcode 的交互面，包括聊天主界面、slash command 体验、会话导航、thinking / tool / subagent 展示与键盘驱动工作流。
- `terminal-chat-read-model`: 定义 server 为终端客户端提供的稳定读模型 / 投影契约，覆盖 transcript、session switch、child agent 状态与终端友好的增量更新语义。

### Modified Capabilities
- `composer-execution-controls`: 执行控制不再只面向当前图形前端 composer，正式终端客户端也必须通过同一稳定合同提交与消费 `compact` 等控制请求。
- `tool-and-skill-discovery`: discovery 结果需要支撑终端 slash palette 与 `/skill` 等命令建议，而不是只隐式服务当前图形输入框。

## Impact

- 影响代码：
  - 新增终端客户端 crate / binary（预计如 `crates/cli`）
  - `crates/server`、`crates/protocol` 中与终端 read model / DTO / SSE 消费相关的 surface
  - 可能扩展 `crates/application` 中面向客户端的稳定查询编排，但不引入新的会话真相
  - 现有前端与桌面端会继续保留，并与终端端共享同一套 server 合同
- 影响依赖：
  - 新增 `ratatui`、`crossterm` 及相关终端事件循环依赖
- 用户可见影响：
  - Astrcode 将拥有一个正式发布的终端版，可覆盖完整聊天工作流，而不只是调试工具
  - 终端端将支持 slash command 补全、会话切换、thinking / 子智能体展示与流式工具输出
- 开发者可见影响：
  - 需要把终端端依赖的 transcript / child-agent / command palette 语义收敛为稳定合同，避免复制前端私有投影逻辑
  - 终端端会成为与桌面端、浏览器端并行的一等 client，需要补充对应的协议测试、契约测试与发布流程

## Non-Goals

- 不在本次变更中替换或下线现有桌面端、浏览器端产品面。
- 不让终端客户端直接依赖 `application` / `kernel` / `session-runtime` 内部结构，也不引入第二套组合根。
- 不先以“能显示消息即可”的最低标准交付；终端端目标是正式发布版本，而不是临时 debug UI。

## Migration / Rollback

本次变更采用增量引入：先增加新的终端入口与其所需的 server 契约，再补充发布链路；现有桌面端和浏览器端保持不变。若终端 read model 或交互面在发布前未达到稳定要求，可以停止发布该 binary 或将新增 surface 保持未启用状态，而无需回滚现有 GUI 路径或破坏既有 session / execution 合同。
