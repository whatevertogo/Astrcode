## ADDED Requirements

### Requirement: `application` 通过治理端口消费运行时协调，而不拥有设施 owner

`application` SHALL 通过治理端口消费进程级运行时协调、治理快照与关闭能力；这些设施 owner 不再由 `core` 持有，也不要求 `application` 自己成为设施 owner。

#### Scenario: application governance does not require core-owned runtime coordinator

- **WHEN** `application` 需要读取治理快照、协调关闭或消费运行时状态
- **THEN** 它 SHALL 通过稳定治理端口完成
- **AND** 不要求直接持有 `RuntimeCoordinator` 这类组合根设施 owner

#### Scenario: application depends on contracts rather than core-owned mutable state

- **WHEN** `application` 需要协调会话运行时、治理快照或关闭行为
- **THEN** 它 SHALL 通过稳定 port 与值对象完成编排
- **AND** 不依赖 `core` 中的全局可变状态 owner

---

### Requirement: `application` 编排项目路径与环境副作用契约，而不直接持有实现

凡是与 project dir、working dir 归一化、tool result durable persist 等环境副作用相关的业务编排，`application` SHALL 依赖稳定契约完成；具体实现 SHALL 留在 adapter 或 `astrcode-support` 这类受限 support crate。

#### Scenario: application does not use core filesystem helpers directly

- **WHEN** 某个应用层用例需要校验 project dir、归一化 working dir 或触发 durable persist
- **THEN** `application` SHALL 通过稳定 port 编排这些能力
- **AND** 不直接调用 `core` 中的具体文件系统 helper
- **AND** 若需要共享宿主路径解析，SHALL 通过 `astrcode-support::hostpaths` 或等价稳定契约消费

#### Scenario: application does not resolve home directories from core

- **WHEN** 应用层需要定位 Astrcode home、project root 或等价宿主路径
- **THEN** 它 SHALL 通过组合根注入的能力、`astrcode-support::hostpaths` 或 adapter 契约完成
- **AND** 不把 `core` 作为 home 目录解析 owner
