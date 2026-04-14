## 1. OpenSpec and Read Model Boundary

- [x] 1.1 新增 `debug-workbench-read-model`、`runtime-observability-pipeline`、`server-http-debug-surface` 对应 specs，并在实现中保持 `server` 只做装配。
- [x] 1.2 新增 `crates/debug-workbench`，封装 runtime overview、timeline、session trace、session agent tree 的读模型与查询 use case。

## 2. Server Debug Surface

- [x] 2.1 扩展 `crates/protocol/src/http/*` 与 `crates/server/src/http/mapper.rs`，补齐 Debug Workbench 所需 DTO。
- [x] 2.2 在 `crates/server/src/http/routes/debug.rs` 与 `routes/mod.rs` 中挂载 `/api/debug/runtime/overview`、`/api/debug/runtime/timeline`、`/api/debug/sessions/{id}/trace`、`/api/debug/sessions/{id}/agents`。
- [x] 2.3 在 debug 构建中为 timeline 提供服务端维护的最近 5 分钟窗口；非 debug 构建不挂载这些路由。

## 3. Frontend Workbench

- [x] 3.1 在 `frontend/` 中新增独立 Debug Workbench 入口与页面，不再在主聊天界面渲染右下角旧浮层。
- [x] 3.2 在 Debug Workbench 中展示全局 overview、最近 5 分钟趋势、当前 session trace 与 agent tree。

## 4. Desktop Window Integration

- [x] 4.1 在 `src-tauri` 中新增 `debug-workbench` 窗口与宿主命令，仅在 debug 模式提供主窗口入口。
- [x] 4.2 确保关闭 Debug Workbench 不影响主窗口会话，重复打开时优先聚焦已存在窗口。

## 5. Validation

- [x] 5.1 补充后端测试，覆盖 debug-only 路由、timeline 窗口裁剪、session trace/agents 不串会话。
- [x] 5.2 补充前端/桌面端测试或最小验证，覆盖独立入口、workbench 渲染和主窗口不再渲染旧浮层。
- [x] 5.3 运行 `cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace`，并在前端改动存在时运行 `cd frontend && npm run typecheck`。
- [x] 5.4 删除旧debug和评估代码路径，确保不再被调用或渲染，保证代码干净
