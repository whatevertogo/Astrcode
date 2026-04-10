# Migration: 删除死代码与兼容层收口

## Migration Principles

- 先做 caller inventory，再删公共入口。
- 无消费者 surface 立即删除，不做“暂存观察期”。
- 活跃主线先迁移，再删 legacy 入口。
- live 文档、测试、夹具与实现必须同一批次收口。

## Caller Inventory

| Surface | Current caller state | Migration action |
|--------|-----------------------|------------------|
| `loadParentChildSummaryList` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 client + tests + docs |
| `loadChildSessionView` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 client + tests + docs |
| `buildParentSummaryProjection` | 无当前 UI 调用；只剩测试/文档引用 | 直接删除 projection + tests + docs |
| `/api/sessions/{id}/children/summary` | 无当前消费者 | 直接删除 route + DTO + tests + docs |
| `/api/sessions/{id}/children/{child_session_id}/view` | 无当前消费者 | 直接删除 route + DTO + tests + docs |
| `/api/v1/agents*` | 无当前产品入口 | 直接删除 route + protocol/doc/tests |
| `/api/v1/tools*` | 无当前产品入口；execute 为骨架 | 直接删除 route + protocol/doc/tests |
| `/api/runtime/plugins*` | 无当前产品入口 | 直接删除 route + tests + docs |
| `/api/config/reload` | 无当前产品入口 | 直接删除 route + tests + docs |
| `cancelSubRun` client + `/subruns/{id}/cancel` route | 当前 UI 仍在调用 | 迁移到 `closeAgent` 后删除 |
| `legacyDurable` / shared-session downgrade public semantics | 当前无正向业务价值，只维持 legacy 显示/测试 | 收敛为明确失败，删除 downgrade surface |

## Phase Order

### Phase 1: 锁定支持面清单

- 完成 caller inventory
- 在 plan/findings/contracts 中标明 retain/remove/migrate-then-remove
- 确认当前主线动作列表

**Exit Criteria**

- 每个候选 surface 都有明确分类
- 没有“再看看”的灰色项

### Phase 2: 删除立即可删的 orphan surface

- 删除 parent-child summary clients / projections / server routes
- 删除 `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload`
- 同步删对应 tests / DTO / docs

**Exit Criteria**

- 被标记为“立即删除”的 surface 在代码、测试、docs 中都不再出现
- 当前会话浏览、当前 child session 直开、当前配置读写不受影响

### Phase 3: 迁移 cancel 主线

- UI cancel 动作切到 `closeAgent`
- 删除 `cancelSubRun` wrapper 与 legacy cancel route
- 改写测试，验证新主线动作

**Exit Criteria**

- 当前“取消子会话”按钮仍可用
- 只剩一条主线 cancel 入口
- `subruns/{id}/cancel` 不再暴露

### Phase 4: 删除 legacy downgrade public semantics

- 删除 `legacyDurable` 公开状态
- 删除 legacy-only subtree / descriptor-missing helper 分支
- 保留明确失败能力与必要错误信息

**Exit Criteria**

- 不再把旧历史包装成“部分可用”
- 前端/协议/运行时不再公开 legacy downgrade surface

### Phase 5: 文档与测试收尾

- 更新 `docs/spec` live 文档
- 删除或改写旧测试/夹具
- 执行完整验证矩阵

**Exit Criteria**

- live 文档只描述保留能力
- tests 只证明保留主线与删除结果

## Checkpoints

### Checkpoint A: Orphan Summary Surfaces Removed

- 前端不再导出 `loadParentChildSummaryList`、`loadChildSessionView`、`buildParentSummaryProjection`
- server 不再暴露对应 children summary/view route
- 保留 `SubRunHandoff.summary` 与 `ChildSessionNotification.summary`

### Checkpoint B: Legacy Cancel Route Removed

- UI 取消动作改用 `closeAgent`
- legacy cancel route 与前端包装被删除
- 主线取消能力通过新入口回归

### Checkpoint C: Public Skeleton Routes Removed

- `/api/v1/agents*`
- `/api/v1/tools*`
- `/api/runtime/plugins*`
- `/api/config/reload`

以上 route、相关 tests 和 live docs 均已删除。

### Checkpoint D: Legacy Downgrade Surface Removed

- `legacyDurable` 不再出现在公开类型、协议 DTO 或前端状态中
- legacy-only tree helper 分支和对应测试被删除
- 旧输入改为明确失败

### Checkpoint E: Live Docs Match Reality

- `docs/spec` 不再宣传已删除 surface
- open items 不再为已决定删除的接口保留伪开放问题

## Validation Matrix

1. `rg -n "loadParentChildSummaryList|loadChildSessionView|buildParentSummaryProjection" frontend crates docs`
2. `rg -n "/api/v1/agents|/api/v1/tools|/api/runtime/plugins|/api/config/reload|subruns/.*/cancel" crates/server frontend docs/spec`
3. `cargo fmt --all --check`
4. `cargo clippy --all-targets --all-features -- -D warnings`
5. `cargo test --workspace --exclude astrcode`
6. `cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`
7. 手工验证：
   - 创建/切换会话
   - 提交消息
   - 聚焦子执行
   - 打开子会话
   - 取消子会话

## Rollback Considerations

- 不建议回滚到“双轨主线 + legacy route”的状态，因为那会重新引入支持面歧义。
- 如果某条被删除的 surface 真有遗漏消费者，应在 caller inventory 层重新登记 owner 与用途，而不是直接恢复骨架实现。
