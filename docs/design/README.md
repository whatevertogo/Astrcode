# Design 文档索引

`design/` 只保留设计结论，不展开成长篇实施计划。

## 文档列表

- [agent-tool-and-api-design.md](./agent-tool-and-api-design.md)  
  子代理系统的主线模型、边界与非目标。
- [runtime-session-and-turn-lifecycle.md](./runtime-session-and-turn-lifecycle.md)  
  Session / Turn / SubRun 的核心真相与职责边界。
- [multi-session-frontend-architecture.md](./multi-session-frontend-architecture.md)  
  多会话前端的导航对象、状态模型与数据来源。
- [runtime-surface-aggregation.md](./runtime-surface-aggregation.md)  
  Runtime surface 聚合对象的分层思路。
- [subagent-session-modes-analysis.md](./subagent-session-modes-analysis.md)  
  子会话模式的采纳边界与控制面原则。
- [compact-system-design.md](./compact-system-design.md)  
  Compact 系统的目标、方向与分层演进思路。

## 与 spec 的关系

- design 讲“为什么这样设计”。
- spec 讲“必须稳定成什么样”。

需要实现细节、字段约束、协议语义时，请继续阅读 `../spec/`。
