## ADDED Requirements

### Requirement: child execution contracts SHALL be emitted from the shared governance assembly path

fresh child 与 resumed child 的 execution contract MUST 由统一治理装配路径生成，而不是由不同调用路径分别手工拼接。

#### Scenario: fresh child contract uses the shared assembly path

- **WHEN** 系统首次启动一个承担新责任分支的 child
- **THEN** child execution contract SHALL 通过共享治理装配器生成
- **AND** SHALL 与同一次提交中的其他治理声明保持同一事实源

#### Scenario: resumed child contract uses the same authoritative source

- **WHEN** 父级复用已有 child 并发送 delta instruction
- **THEN** resumed child contract SHALL 由同一治理装配路径生成
- **AND** SHALL NOT 退回到独立 helper 拼接的平行实现

### Requirement: delegation catalog and child contracts SHALL stay consistent under the same governance surface

delegation catalog 可见的 behavior template、child execution contract 中的责任边界与 capability-aware 限制 MUST 来源于同一治理包络。

#### Scenario: catalog and contract agree on branch constraints

- **WHEN** 某个 child template 在当前提交中可见且被用于启动 child
- **THEN** delegation catalog 与最终 child execution contract SHALL 体现一致的责任边界和限制摘要
- **AND** SHALL NOT 让 catalog 与 contract 分别读取不同来源的治理事实

### Requirement: collaboration facts SHALL be recordable with governance envelope context

`AgentCollaborationFact`（core/agent/mod.rs:1129-1155）记录 spawn/send/observe/close/delivery 等协作动作的审计事件。这些事实 MUST 能关联到生成该动作时的治理包络上下文，使审计链路可追溯。

#### Scenario: collaboration fact includes governance context

- **WHEN** 系统记录一个 `AgentCollaborationFact`（如 spawn 或 send）
- **THEN** 该事实 SHALL 能关联到当前 turn 的治理包络标识或摘要
- **AND** SHALL NOT 丢失治理上下文导致无法追溯决策依据

#### Scenario: policy revision aligns with governance envelope

- **WHEN** `AGENT_COLLABORATION_POLICY_REVISION` 用于标记协作策略版本
- **THEN** 该版本标识 SHALL 与治理包络中的策略版本一致
- **AND** SHALL NOT 出现审计事实的策略版本与实际治理策略不同步

### Requirement: CollaborationFactRecord SHALL derive its parameters from the governance envelope

`CollaborationFactRecord`（agent/mod.rs:96-166）跟踪每个协作动作的结果、原因码和延迟。其构建参数 MUST 来自治理包络，而不是各调用点独立组装。

#### Scenario: fact record uses governance-resolved child identity and limits

- **WHEN** 系统为一个 spawn 或 send 动作构建 `CollaborationFactRecord`
- **THEN** child identity、capability limits 等字段 SHALL 从治理包络中获取
- **AND** SHALL NOT 从不同参数源独立读取导致与治理包络不一致
