# Quickstart: 删除死代码与冗余契约收口

本文件用于实现后快速验证"主线仍在、冗余已删、兼容层已收口"。

## 1. 静态检查

确认以下 orphan surface 与 duplicated contract 已消失：

```powershell
# 检查 1: 核心类型已删除（只在注释中提及替代关系属于合规）
rg -n "SubRunDescriptor\b" crates/core crates/runtime crates/runtime-execution crates/runtime-agent-control
# 期望：0 命中（注释引用除外）

rg -n "SubRunOutcome" crates/core crates/runtime crates/runtime-execution crates/runtime-agent-control
# 期望：0 命中（注释引用除外）

rg -n "PromptAccepted|RootExecutionAccepted|AgentExecutionAccepted" crates/core crates/runtime crates/runtime-execution crates/runtime-agent-control
# 期望：0 命中（注释引用除外）

# 检查 2: 前端 orphan surface 已删除
rg -n "loadParentChildSummaryList|loadChildSessionView|buildParentSummaryProjection|ParentChildSummaryListResponseDto|ChildSessionViewResponseDto" frontend crates/protocol crates/server
# 期望：0 命中

# 检查 3: ChildAgentRef.openable 字段已删除
rg -n "\bopenable\b" crates/core crates/protocol crates/server
# 期望：0 命中

# 检查 4: protocol status 不再是 String
rg -n "status: String" crates/protocol/src
# 期望：0 命中

# 检查 5: 旧 cancel route 不再存在（仅测试拒绝旧路由时引用）
rg -n "subruns/.*/cancel" crates/server/src/http/routes
# 期望：0 命中（测试文件中的引用除外）

# 检查 6: open_session_id 只保留在 canonical child ref
rg -n "pub open_session_id" crates/core crates/protocol crates/server
# 期望：仅在 agent.rs (protocol child ref DTO) 和 core/agent/mod.rs (canonical child ref)

# 检查 7: legacy_shared_history_error_fixture 已删除
rg -n "legacy_shared_history_error_fixture" crates/runtime-agent-loop
# 期望：0 命中
```

## 2. 自动化验证

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
cd frontend
npm run typecheck
npm run lint
npm run format:check
npm run test
```

## 3. 手工场景

### 场景 A：当前会话主线仍可用

1. 打开应用
2. 切换到任意会话
3. 提交一条消息
4. 确认历史与 SSE 增量正常

### 场景 B：当前子执行聚焦仍可用

1. 触发一个子执行
2. 进入 focused subrun 视图
3. 确认 breadcrumb、内容区和返回主会话动作正常

### 场景 C：当前 child session 直开仍可用

1. 触发独立子会话
2. 从当前 UI 打开 child session
3. 确认能够直接进入 `child_ref.open_session_id` 指向的目标会话

### 场景 D：关闭子会话走唯一主线

1. 触发一个后台运行的子会话
2. 点击"取消子会话 / 关闭子 agent"
3. 确认动作成功，且实现走 `closeAgent` 主线，而不是旧 cancel route

### 场景 E：legacy 输入明确失败

1. 准备一份缺少 parentTurnId 的旧样本
2. 让它进入当前 subrun 浏览流程
3. 确认系统给出明确失败，而不是构建 downgrade 视图

### 场景 F：文档不再宣传旧 surface

```powershell
rg -n "cancel route" docs/spec
```

期望：live 文档不再把旧 cancel route 写成当前事实。

### 场景 G：protocol 状态与 metrics 合同收口

1. 触发一个 child/subrun 状态变化
2. 检查协议载荷，确认状态字段为枚举值（`AgentStatus`）而不是任意字符串
3. 检查 prompt metrics 相关载荷，确认共享指标字段来自 `PromptMetricsPayload` 唯一定义
