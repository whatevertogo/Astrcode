## Context

当前仓库已经把旧 `runtime` 巨石拆成 `application + kernel + session-runtime + adapter-* + server` 的长期边界，但治理链路仍然处于半迁移状态：

- `AppGovernance` 已存在，但组合根没有为其接入真正的 `reloader`
- `POST /api/config/reload` 只执行配置重读，不执行完整 surface 刷新
- MCP 自带一条局部 reload/sync 路径，plugin 与 builtin 刷新没有统一编排入口
- 治理快照和 reload 结果还不能稳定表达“本次刷新后系统真正生效的 capability surface 是什么”

这会导致一个典型问题：用户看到“reload 成功”，但实际上只有部分来源被刷新，系统处于事实分叉状态。对于强调 Server is the truth 的架构，这种半刷新比显式失败更危险。

## Goals / Non-Goals

**Goals:**

- 为 `application` 建立唯一治理 reload 入口，统一编排 builtin、MCP、plugin 三类能力来源
- 明确 reload 的原子性边界、失败语义、回滚策略和运行中会话限制
- 让 server 路由只消费 `AppGovernance` 暴露的治理合同，不再散落局部 reload 逻辑
- 让治理快照与 reload 结果稳定反映当前 capability surface 和 plugin 生命周期状态

**Non-Goals:**

- 不重写 `kernel` 的 capability router 或 `session-runtime` 的 turn 执行主流程
- 不增加新的前端治理页面
- 不保留旧 `runtime` reload 行为的长期兼容分支

## Decisions

### 决策 1：把 reload 定义为治理级用例，而不是配置服务副作用

`ConfigService::reload_from_disk` 只负责返回新的配置事实；完整 reload 由 `AppGovernance` 通过 `RuntimeReloader` 统一编排。这样可以让“重读配置”和“重建运行时能力面”成为两个清晰概念。

选择这个方案，而不是继续在 `server/http/routes/config.rs` 里直接拼 reload，是因为：

- `server` 只应承载 transport concern，不应重新长出业务编排
- `application` 已经是治理入口，reload 语义应与 shutdown、snapshot 处于同一层
- 配置读取失败与 surface 替换失败是不同故障，不应该被压扁成同一种“reload 失败”

备选方案是继续保留 `config reload` 与 `mcp reload` 两条链路，然后在 server 层做协调；该方案被放弃，因为它会继续制造多入口和状态竞争。

### 决策 2：reload 采用“先组装候选 surface，再一次性替换”的原子模型

新的治理 reload 按以下顺序工作：

1. 读取并校验新配置
2. 基于新配置重新发现 plugin / MCP / builtin 能力输入
3. 在替换前组装完整候选 capability surface 与对应治理快照数据
4. 仅当候选结果可用时，一次性替换 `kernel` 当前 surface 与治理侧快照依赖
5. 替换后关闭旧的托管组件

这意味着 reload 失败时必须保留旧 surface 继续服务，不能出现“plugin 刷新了一半、MCP 已换、builtin 还是旧的”这种状态。

备选方案是增量变更当前 surface；该方案被放弃，因为增量替换难以定义失败回退边界，也会让 capability 可见性在刷新期间不断漂移。

### 决策 3：运行中会话一律阻止治理级 reload

治理级 reload 将显式检查是否存在运行中 session；只要存在活跃执行，就拒绝本次 reload，并返回业务错误与可观测原因。

选择显式拒绝，而不是“尽量刷新不影响运行中的部分”，原因是：

- `session-runtime` 的执行中会绑定当前可见的 capability surface
- 中途替换 surface 可能让 tool、resource、plugin、MCP 可见性失真
- 当前架构优先一致性和可解释性，不追求热更新的最大并发度

未来若要支持更细粒度 live reload，应作为新能力单独设计，而不是在当前治理模型里偷偷放宽。

### 决策 4：MCP 局部刷新纳入治理语义，但保留其实现层专用装配

`adapter-mcp` 仍然可以保留自身的连接管理和配置解析实现，但对外暴露的“重载已完成”语义必须统一经过治理入口。

具体来说：

- MCP 端口层仍可负责把配置变化转换为最新 invoker surface
- 但 capability sync、治理快照更新、失败表达与旧状态保留由统一 reload 语义约束
- server 不能再把 MCP reload 当作独立于治理面之外的“隐式成功”

这样做的理由是保留实现分层：MCP 的 transport/manager 细节仍属于 adapter，但系统语义属于 application governance。

### 决策 5：治理快照以 application 暴露的稳定 DTO 为准，protocol 只做映射

治理快照的事实源保持在 `application`，server 只做 DTO 映射。任何 plugin 状态、capability surface、reload 结果的业务解释都不得下沉到 protocol 或 HTTP mapper 中。

这与当前项目架构一致，也避免未来为了某个返回字段再把治理语义塞回 server。

## Risks / Trade-offs

- [Risk] reload 统一后，原先可局部成功的 MCP 操作会变得更严格 → Mitigation：把“局部配置编辑成功”和“治理级刷新成功”拆成不同结果，失败时明确说明旧状态仍生效
- [Risk] 运行中会话阻止 reload 可能影响开发体验 → Mitigation：在错误中返回运行中 session 标识，并把该限制写入 spec、UI 提示和测试
- [Risk] 候选 surface 组装阶段可能带来额外启动/刷新延迟 → Mitigation：保持组装阶段纯内存构建，避免在替换后再做昂贵回滚
- [Risk] 旧路径仍被调用导致双入口共存 → Mitigation：在迁移计划中优先把所有 reload 路径重定向到统一治理入口，再删除局部入口的成功语义

## Migration Plan

1. 为 `AppGovernance` 接入真正的 `RuntimeReloader` 实现，并补齐 reload 结果模型
2. 将 `server` 的配置重载路由改为调用治理 reload，而不是仅调用配置服务
3. 让 MCP 刷新路径在成功/失败表达上对齐治理语义，避免 server 侧绕过治理面
4. 调整治理快照组装方式，让 plugin/capability 状态基于当前生效 surface 输出
5. 删除旧的分叉 reload 成功路径，并补齐回归测试

回滚策略：

- 若候选 surface 组装失败，则直接保留旧 surface 并返回显式失败
- 若替换后关闭旧组件失败，只记录治理失败并保持新 surface 已生效，不回滚到半旧状态

## Open Questions

- `POST /api/config/reload` 是否继续沿用原路径但升级语义，还是后续增加更明确的治理命名端点
- reload 结果是否需要在同一响应里同时返回“新配置视图 + 新治理快照”，还是拆成两个读取接口
- plugin health probe 的即时刷新是否属于本次 reload 同步阶段，还是作为 reload 后的异步补全步骤
