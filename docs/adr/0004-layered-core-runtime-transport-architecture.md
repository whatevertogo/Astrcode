# ADR-0004: Freeze Layered Core, Runtime, and Transport Boundaries

- Status: Accepted
- Date: 2026-03-30

## Context

随着 AgentLoop、capability routing、prompt contributor 和 plugin lifecycle 逐步稳定，AstrCode 的主要风险已从“能力不足”转为“核心语义、运行时装配和传输适配彼此混杂”。如果不冻结分层边界，后续继续扩展审批、skills、agents、多 provider 和多 transport 时，职责会继续漂移。

## Decision

AstrCode 采用三层架构边界：核心契约层、运行时装配层、传输适配层。

- 核心契约层只包含稳定的协议 DTO 与行为契约，例如 capability、policy、event、tool、session 等抽象；不包含具体 provider、插件宿主、存储实现或传输适配。
- 运行时装配层负责把核心契约组装为可运行系统，包括 provider、prompt、storage、tool、plugin 和 policy 等实现。
- 传输适配层只负责 HTTP、SSE、桌面壳和前端等接入形式，不拥有核心业务语义，也不主导运行时装配。
- 核心契约的演进应优先发生在核心层；具体实现变更优先落在运行时装配层或传输层，而不是让 transport 反向定义核心抽象。

## Consequences

- 核心边界更稳定，transport 不再天然拥有业务装配权。
- runtime 明确承担框架化装配职责，server 和桌面壳可以保持更薄。
- 模块职责更清晰，但边界需要持续靠测试和文档约束维持。
