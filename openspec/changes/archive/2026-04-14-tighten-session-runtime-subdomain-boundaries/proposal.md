## Why

`application` 与 `session-runtime` 的总分层已经基本稳定，但 `session-runtime` 内部子域边界仍然偏松，`application` 里也残留了少量会话内推进细节。继续放任这些职责漂移，会让 `context`、`context_window`、`query`、`observe`、`factory` 等模块逐渐重叠，重新长回“杂货铺式 runtime”。

现在推进这次收口，是因为我们已经完成了一轮 `application -> session-runtime` 的职责下沉，正处在最适合立清楚内部边界、避免后续继续混写的窗口期。若不尽快把子域职责钉牢，后续任何 request assembly、observe 视图或 query 扩展都会再次把边界拉糊。

## What Changes

- 收紧 `session-runtime` 内部子域边界，明确：
  - `context` 只负责上下文来源、继承与解析结果
  - `context_window` 只负责预算、裁剪、压缩与窗口化消息序列
  - `actor` 只负责推进与持有单 session live truth
  - `observe` 只负责推送/订阅语义与过滤范围
  - `query` 只负责拉取、快照与投影
  - `factory` 只负责构造执行输入或执行对象
- 把 `context_window` 中与最终 request assembly 绑定过深的职责迁出，归到更中性的 request/prompt assembly 子域。
- 继续把 `application` 中残余的单 session 查询、终态判定、durable append 细节下沉到 `session-runtime`，保留 `application` 作为薄用例门面与跨 session 编排层。
- 将 `query` 从单文件大模块拆分为按职责分块的子模块，至少覆盖 `history`、`agent`、`mailbox`、`turn` 四类读取场景。
- 更新架构文档与相关 spec，明确这些子域的 allowed responsibilities、forbidden responsibilities、迁移边界与后续扩展规则。

## Capabilities

### New Capabilities

- `session-runtime-subdomain-boundaries`: 约束 `session-runtime` 内部子域的职责边界、命名语义与禁止事项，避免 `context` / `context_window`、`actor` / `observe` / `query`、`factory` 继续相互渗透。

### Modified Capabilities

- `session-runtime`: 调整 `session-runtime` 的内部职责分块要求，要求 query、context、request assembly、mailbox append 等能力按新的子域边界收口。
- `application-use-cases`: 调整 `application` 对 `session-runtime` 的依赖方式，要求 `application` 不再保留单 session 查询、终态投影与 durable append 细节。

## Impact

- 影响代码：
  - `crates/session-runtime/src/context*`
  - `crates/session-runtime/src/query/*`
  - `crates/session-runtime/src/factory/*`
  - `crates/session-runtime/src/observe/*`
  - `crates/application/src/agent/*`
- 不修改 HTTP API、SSE DTO、工具参数或 `App` 的公开业务接口。
- 会新增或调整 `session-runtime` 的内部稳定查询/命令 API，并相应补充单测与回归测试。
- 用户可见影响较小，主要收益是后续功能扩展时更稳定；开发者可见影响较大，需要按新的子域规则放置实现与测试。

## Non-Goals

- 不改仓库总分层，不引入新的大一统 façade。
- 不调整 `watch`、`config`、`mcp` 的归属。
- 不把跨 session 的 `wake` 主编排迁入 `session-runtime`。
- 不为了兼容旧内部模块路径保留额外过渡层。

## Migration And Rollback

- 迁移方式采用渐进式下沉：先补 `session-runtime` 新 API，再迁移 `application` 调用点，最后删除旧模块与旧直接访问路径。
- 若中途发现某类编排逻辑无法自然下沉，则优先保留在 `application`，并补充 design 说明，而不是把跨 session 协调硬塞回 `session-runtime`。
- 回滚时只需恢复 `application` 对旧内部模块的调用路径，不涉及外部 API 与持久化格式回滚。
