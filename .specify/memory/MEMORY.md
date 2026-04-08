# Memory Index

## Architecture
- [Runtime Boundary Architecture](runtime_boundary_architecture.md) — 五边界单一职责、单向编译依赖、真相来源不可混淆
- [Subrun Durable Lineage Protocol](subrun_durable_lineage.md) — Durable history = 已完成 subrun 唯一真相、descriptor/tool_call_id 强制写入、legacy 不伪造
- [Execution Lineage Index](execution_lineage_index.md) — 从 descriptor 构建索引、三条路径一致、不推断 ancestry
- [Working-Dir Execution Context](working_dir_execution_context.md) — Agent 解析绑定执行上下文 working directory、不回退进程默认

## Code Quality
- [Code Quality Fixes](code_quality_fixes.md) — 锁恢复机制、错误链保留、异步任务管理、日志级别、Plugin 类型归属、模块大小约束
- [Tool Security Enhancements](tool_security_enhancements.md) — 设备文件黑名单、UNC 路径检查、文件大小限制、符号链接检测、引号规范化、Grep 限制
