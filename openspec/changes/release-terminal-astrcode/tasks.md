## 1. Conversation v1 协议骨架

- [x] 1.1 在 `crates/protocol/src/http/` 下新增 `terminal/` 模块与 `v1` DTO 命名空间，定义 snapshot、stream delta、cursor、terminal block、child summary、slash candidate 与 terminal error envelope。
- [x] 1.2 为 terminal block 建立稳定类型集合，至少覆盖 `user`、`assistant`、`thinking`、`tool_call`、`tool_stream`、`error`、`system_note/compact`、`child_handoff`，并明确哪些错误进入 transcript、哪些只走 banner/status。
- [x] 1.3 在 `crates/protocol/tests/fixtures/` 与 `crates/protocol/tests/` 增加 conversation v1 fixture/conformance tests，冻结 `/api/v1/conversation/*` 的 JSON 形状、delta kind、opaque cursor 与错误 envelope 行为。

## 2. Client Facade 与错误模型

- [x] 2.1 新增 `crates/client/` crate，提供纯 typed facade：`exchange_auth`、`list_sessions`、`create_session`、`submit_prompt`、`request_compact`、`fetch_terminal_snapshot`、`stream_terminal`、`list_slash_candidates`。
- [x] 2.2 在 `crates/client/src/` 落地统一错误模型与映射，至少覆盖 `auth/permission`、`validation`、`not_found/conflict`、`stream_disconnected`、`cursor_expired`、`transport_unavailable`，禁止把裸 `reqwest::Error` 直接泄漏到 TUI。
- [x] 2.3 在 `crates/client/src/` 实现 terminal stream 的 SSE 消费、cursor catch-up、bounded channel 与 lagged/rehydrate-required 语义，确保 `client` 只连接既有 `origin/token`，不承担 spawn 或 server 发现职责。
- [x] 2.4 为 `crates/client/` 增加 mock transport 测试，覆盖 auth exchange、snapshot 拉取、stream catch-up、cursor 失效、token 过期与结构化错误归一化。

## 3. Launcher 边界与启动策略

- [x] 3.1 在 `crates/cli/src/launcher/` 收口 launcher 边界，负责 `--server-origin` / `--token`、`~/.astrcode/run.json`、本地 `astrcode-server` spawn、ready handshake 与子进程生命周期管理。
- [x] 3.2 明确 launcher 输出的稳定连接结果类型（如 resolved `origin/token/working_dir/source`），让 `crates/client/` 只消费该结果，不感知 managed-local 分支。
- [x] 3.3 为 `crates/cli/src/launcher/` 增加测试或最小 harness，覆盖 attach 已运行 server、spawn 本地 server、ready 握手失败与远程 token 无效四条主路径。

## 4. Application 查询编排

- [x] 4.1 在 `crates/application/src/` 新增 terminal 用例子域，定义 surface-neutral 的 `TerminalFacts`、snapshot query、stream catch-up query、session resume query 与 child summary query。
- [x] 4.2 让 terminal 用例只依赖 `session-runtime` 的稳定 history/replay/observe/child lineage 查询接口，禁止读取 `session-runtime` 内部投影细节或把 `protocol` DTO 带回 `application`。
- [x] 4.3 在 terminal 用例中落实 control state、active session、slash candidate 所需的编排与权限校验，确保 `/compact`、`/resume`、`/skill` 的读侧事实统一来自 server。
- [x] 4.4 为 `crates/application/src/` 的 terminal 用例增加单元测试，覆盖 snapshot hydration、cursor catch-up、cursor 失效回退、resume search 与 child 可见性边界。

## 5. Server 侧 Terminal Projection

- [x] 5.1 在 `crates/server/src/http/` 下新增 `terminal_projection` 模块，把 `TerminalFacts` 纯映射到 `protocol::http::terminal::v1::*`，保持无状态、可测试，不夹带业务校验。
- [x] 5.2 在 terminal projection 中实现 event/block 聚合规则，确保 `thinking`、`tool_stream`、`child_handoff`、turn-scoped `error` 与 control state 具备稳定 block id、终态与 patch 语义。
- [x] 5.3 为 `crates/server/src/http/terminal_projection*` 增加 fixture/snapshot 风格测试，冻结 `TerminalFacts -> TerminalBlock/Delta` 映射，避免 GUI/terminal 语义再次漂移。

## 6. Server Route 与 Conversation v1 Surface

- [x] 6.1 在 `crates/server/src/http/routes/` 下新增 conversation v1 路由，至少暴露 `GET /api/v1/conversation/sessions/{id}/snapshot` 与 `GET /api/v1/conversation/sessions/{id}/stream?cursor=...`，并接入现有 auth 机制。
- [x] 6.2 为 conversation v1 route 增加明确的结构化错误返回，区分 `rehydrate_required`、`auth_expired`、`not_found`、`forbidden` 与参数校验失败，不把 legacy `/events` 的语义隐式复用过来。
- [x] 6.3 在 `crates/server/src/http/routes/mod.rs`、相关 mapper 与 observability 接线中补齐 conversation route 注册、SSE framing、cursor 解析与 catch-up 指标记录。
- [x] 6.4 为 `crates/server/src/tests/` 增加 conversation route 集成测试，覆盖 snapshot hydration、stream catch-up、cursor 失效、active session 切换与 route 不依赖 legacy `/events`。

## 7. Discovery 与 Execution Control 接线

- [x] 7.1 在 `crates/application/src/composer/`、`crates/server/src/http/routes/composer.rs` 或相邻路径补齐 terminal slash candidate 所需字段，至少包含候选 id、title、description、keywords、insert_text/command action。
- [x] 7.2 调整 `composer-execution-controls` 相关链路，确保 terminal `/compact` 通过显式 control contract 提交，并且 busy-session 下的 control state 可跨 reconnect / surface switch 观察。
- [x] 7.3 补充 discovery 与 execution control 的契约测试，验证 slash suggestion 不依赖本地注册表、不可用候选不会暴露、control state 以 server 事实为准。

## 8. CLI 主循环与状态模型

- [x] 8.1 在 `crates/cli/src/app/`、`state/`、`command/`、`render/`、`ui/` 建立 TUI 主循环骨架，串起 launcher、client、tick、input、action dispatch 与渲染状态。
- [x] 8.2 在 `crates/cli/src/state/` 建立 terminal block 状态、scroll anchor、pane focus、active session、resume search 与 banner/status 错误模型，明确 transcript error block 与连接级错误的分流。
- [x] 8.3 在 `crates/cli/src/command/` 实现 `/new`、`/resume`、`/compact`、`/skill` 的解析、候选选择与动作路由，禁止在 CLI 本地写平行业务语义。
- [x] 8.4 在 `crates/cli/src/app/` 中落实“v1 仅一个 active session live stream”的策略，确保 session 切换时正确取消旧 stream、重新 hydrate 新 session，并保持 resume/list 查询仍可工作。

## 9. Transcript、Pane 与渲染策略

- [ ] 9.1 在 `crates/cli/src/ui/` 与 `render/` 实现 transcript、child pane/focus view、status/banner、slash palette 的基础组件，覆盖 thinking、tool stream、child handoff、turn error block 与 compact/system note。
- [ ] 9.2 在 `crates/cli/src/render/` 实现 resize 处理：窗口变化时失效 line-wrap cache、重算 scroll anchor 与 child pane 布局，禁止继续复用旧宽度下的渲染缓存。
- [ ] 9.3 在 `crates/cli/src/app/` / `render/` 加入 `Smooth` 与 `CatchUp` 两档 stream chunking 策略，基于队列深度与最老 chunk age 切换，避免高吞吐流式输出打爆终端渲染。
- [ ] 9.4 在 `crates/cli/src/capability/` 实现 truecolor、unicode width、alt-screen、mouse、bracketed paste 探测与 degrade 策略，保证最差情况下仍能退化到 ASCII + no-color 的基本聊天体验。

## 10. 端到端验证与发布接线

- [ ] 10.1 为 `crates/cli/` 增加 ratatui test backend 渲染测试，至少覆盖 transcript、child pane、slash palette、error/banner、空状态与 degrade 模式。
- [ ] 10.2 增加 server + client + cli 的端到端验收脚本或集成测试，覆盖 attach 已运行 server、managed-local 启动、snapshot/stream、resume、/compact、/skill 与单 active stream 切换。
- [ ] 10.3 更新 `PROJECT_ARCHITECTURE.md`、必要的开发文档与 release 说明，明确 `launcher / client / terminal_projection / server route / tui app` 边界，以及 conversation v1 已替代并删除 legacy `/view`/`history`/`events` 产品读面。
- [ ] 10.4 将 `astrcode-cli` 纳入构建与发布产物，补齐验证命令：`cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace`、`node scripts/check-crate-boundaries.mjs`，并在需要时补充 frontend 验证以确认现有 surface 未回归。
