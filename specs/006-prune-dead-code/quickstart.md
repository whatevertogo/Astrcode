# Quickstart: 删除死代码与兼容层收口

本文件用于实现后快速验证“主线仍在、死代码已删、兼容层已收口”。

## 1. 静态检查

确认以下孤儿 surface 已消失：

```powershell
rg -n "loadParentChildSummaryList|loadChildSessionView|buildParentSummaryProjection" frontend crates docs
rg -n "/api/v1/agents|/api/v1/tools|/api/runtime/plugins|/api/config/reload|subruns/.*/cancel" crates/server frontend docs/spec
rg -n "legacyDurable|ParentSummaryProjection|ChildSummaryCard" frontend crates/protocol crates/server docs/spec
```

期望：以上命令不再命中已删除 surface；若有命中，必须能说明它属于 archive 或当前验证说明。

## 2. 自动化验证

```powershell
cargo fmt --all --check
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
3. 确认能够直接进入目标会话

### 场景 D：取消子会话走新主线

1. 触发一个后台运行的子会话
2. 点击“取消子会话”
3. 确认动作成功，且实现走 `closeAgent` 主线，而不是旧 cancel route

### 场景 E：文档不再宣传旧 surface

```powershell
rg -n "SharedSession 为正式路径|IndependentSession 为 experimental|/api/v1/agents|/api/v1/tools|/api/runtime/plugins|/api/config/reload" docs/spec
```

期望：live 文档不再把这些内容写成当前事实。
