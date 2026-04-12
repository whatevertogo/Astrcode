## Context

当前仓库已经把单 session 真相收回 `session-runtime`，也已经有 `kernel::agent_tree` 作为全局控制面基础，但真正的业务执行入口仍然缺失：

- `execute_root_agent` 还没有正式落到 `application`
- 子代理执行的完整桥接还没有形成清晰子域
- working-dir 级 profile 解析与缓存还没有稳定出口

如果继续把这些逻辑零散地塞进 `App` 根文件、server routes 或 `kernel`，会很快破坏刚刚建立好的架构边界。

## Goals / Non-Goals

**Goals:**

- 在 `application` 中建立独立 `execution` 子域
- 实现根代理执行和子代理执行的正式业务入口
- 为 working-dir 级 agent profile 解析与缓存建立稳定接口
- 让 server routes 继续只依赖 `App` 或其稳定服务接口

**Non-Goals:**

- 不把 profile loader 实现细节放进 `kernel`
- 不把权限、审批、profile 解析下沉到 `session-runtime`
- 不直接修改 protocol DTO 结构
- 不在本次 change 中完成 plugin 生命周期接线

## Decisions

### D1: 执行入口单独放进 `application/execution`

根执行和子代理执行都是业务用例，而不是 session 真相或全局控制，因此应当进入 `application/execution` 子域，而不是继续留在 `App` 根文件。  
备选方案是继续在 `App` 上平铺方法，但这样会让 `App` 重新变成大门面。

### D2: `application` 负责解析 profile，`session-runtime` 只消费已解析输入

agent profile 的选择、working-dir 级缓存、权限与策略判断属于业务入口，因此放在 `application`。  
`session-runtime` 只接收已经解析好的执行输入，不拥有 profile 解析职责。  
备选方案是把 profile 解析放进 `session-runtime`，但这会再次污染单 session 真相面。

### D3: `kernel` 只提供全局 agent control，不直接发起业务执行

`kernel` 提供 subrun handle、取消传播、观察、树结构约束等全局能力；真正的 `execute_root_agent` / `launch_subagent` 由 `application` 编排。  
这样可以避免 `kernel` 重新变成“既控制、又编排、又执行业务”的中心层。

### D4: 子代理结果回流走现有 control/session 边界

子代理执行完成后的结果回流通过现有的 agent control / delivery 机制接回父级，而不是在 `application` 自己保存一份临时结果目录。  
这样可以保持“全局控制留在 kernel，单 session 执行留在 session-runtime，业务入口留在 application”的职责闭环。

## Risks / Trade-offs

- [Risk] 执行子域引入后 `App` 与 `execution` 的边界仍不清晰
  - Mitigation：`App` 只作为 façade，把具体实现委托给 `execution` 子模块
- [Risk] profile 缓存策略不当导致 working-dir 变更后读到旧值
  - Mitigation：缓存键基于规范化 working-dir，并预留显式失效入口
- [Risk] 子代理执行桥接跨越多个模块，容易再次形成隐式耦合
  - Mitigation：在 design/spec 中明确输入输出边界和所有权，不让 `application` 保存 session 真相
- [Trade-off] 新增 `application/execution` 会让模块数增加
  - Mitigation：以模块数换取职责清晰度，避免 `App` 根文件继续膨胀

## Migration Plan

1. 建立 `application/execution` 子域与最小 façade
2. 落地 `execute_root_agent`
3. 落地 `launch_subagent` 与结果回流
4. 建立 profile 解析与缓存
5. 让 server agent routes 切到新用例入口

回滚策略：

- 若执行子域尚不稳定，可暂时让 `App` 保留薄 façade 调用旧路径
- 但不恢复旧 runtime façade，也不把 profile 解析塞回 `kernel/session-runtime`

## Open Questions

- `application/execution` 最终是否需要进一步拆成 `root.rs` / `subagent.rs` / `profiles.rs` 三类实现文件？
- 子代理执行结果是否需要额外稳定查询接口，还是现有 subrun status + delivery 已足够？
