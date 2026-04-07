---
name: Working-Dir Execution Context
description: Agent resolution binds to execution working directory, not process-level default
type: execution-model
---

# Working-Dir Execution Context

## 核心原则 (不可违反)

Agent 定义解析与 watch scope 跟随 `ExecutionResolutionContext` 的 working directory，**不回退到进程级默认值**

## 职责分离

- `runtime-agent-loader`: 纯 loader，"怎么读文件"
- `WorkingDirAgentResolver`: 在 `runtime` facade 层，"这次 execution 绑定哪份快照"
- Watch/caching: Runtime 生命周期管理，cache key = working directory
