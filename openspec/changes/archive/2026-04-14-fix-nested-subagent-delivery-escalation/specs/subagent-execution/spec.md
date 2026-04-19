## ADDED Requirements

### Requirement: child turn terminal result MUST use a unified finalizer

`application` SHALL 使用统一的 child turn terminal finalizer 收口真正的 child work turn 的 terminal 结果，而不是按 spawn、resume 分散维护不同逻辑。

#### Scenario: spawn child turn reaches terminal

- **WHEN** child agent 的首轮 spawn turn 结束
- **THEN** 系统 MUST 通过统一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

#### Scenario: resumed child turn reaches terminal

- **WHEN** child agent 通过 idle-resume 继续执行并结束
- **THEN** 系统 MUST 通过同一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

### Requirement: wake turn MUST NOT auto-manufacture a new upward terminal delivery

parent-delivery wake turn 是消费 mailbox 的协调 turn，不属于新的 child work turn。

#### Scenario: wake turn reaches terminal

- **WHEN** child agent 因 parent-delivery wake 而开始新一轮 turn 并结束
- **THEN** 系统 MUST 只完成当前 mailbox batch 的 `acked / consume / requeue`
- **AND** MUST NOT 因为这个 wake turn 自动向更上一级写入新的 terminal delivery

### Requirement: terminal business failures MUST still be delivered upward

child turn 的业务终态若为 `Failed`、`Cancelled` 或 `TokenExceeded`，系统 SHALL 仍将其作为 terminal delivery 投递给直接父级。

#### Scenario: child turn fails

- **WHEN** child turn 进入 `Failed`
- **THEN** 系统 MUST 生成失败态 terminal delivery
- **AND** 直接父级 MUST 能观察到该失败投影

#### Scenario: child turn is cancelled

- **WHEN** child turn 进入 `Cancelled`
- **THEN** 系统 MUST 生成关闭态 terminal delivery
- **AND** 直接父级 MUST 能观察到该关闭投影

### Requirement: finalizer failures MUST NOT fake successful consumption

如果统一 finalizer 自身失败，系统 SHALL 保持当前交付批次可重试，不得制造“上级已经成功收到结果”的假象。

#### Scenario: finalizer append fails

- **WHEN** finalizer 在追加 durable notification 之前或期间失败
- **THEN** 系统 MUST NOT 标记对应批次为已成功消费
- **AND** 上级 MUST 保留后续重试机会
