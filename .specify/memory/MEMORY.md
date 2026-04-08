# Memory Index

## Architecture
- [Runtime Boundary Architecture](runtime_boundary_architecture.md) — 五边界单一职责、单向编译依赖、真相来源不可混淆
- [Subrun Durable Lineage Protocol](subrun_durable_lineage.md) — Durable history = 已完成 subrun 唯一真相、descriptor/tool_call_id 强制写入、legacy 不伪造
- [Execution Lineage Index](execution_lineage_index.md) — 从 descriptor 构建索引、三条路径一致、不推断 ancestry
- [Working-Dir Execution Context](working_dir_execution_context.md) — Agent 解析绑定执行上下文 working directory、不回退进程默认

## 003-subagent-child-sessions
- [子 Agent 独立会话架构](child_session_architecture.md) — ChildSessionNode/ChildAgentRef/Notification 数据模型、所有权边界、durable 真相原则
- [子 Agent 协作工具契约](collaboration_tools.md) — 六工具族（spawn/send/wait/close/resume/deliver）、约束与 runtime inbox 映射、幂等去重
- [父子会话双层投影视图](parent_child_projection.md) — 父侧摘要卡片 + 子侧直开完整 session、SubRunThreadTree 降级 legacy、三层交互规则
- [子 Agent 会话迁移计划](child_session_migration.md) — 五阶段 M1~M5 迁移计划与完成状态、验证命令
- [Anthropic 缓存优化](anthropic_cache_optimization.md) — Phase 1 可见性、Phase 2 消息缓存深度 3、Phase 3 CacheTracker 失效检测

## Code Quality
- [Code Quality Fixes](code_quality_fixes.md) — 锁恢复机制、错误链保留、异步任务管理、日志级别、Plugin 类型归属、模块大小约束
- [Tool Security Enhancements](tool_security_enhancements.md) — 设备文件黑名单、UNC 路径检查、文件大小限制、符号链接检测、引号规范化、Grep 限制
