---
name: Execution Lineage Index
description: Unified scope filtering from durable descriptor, no ancestry inference
type: query-model
---

# Execution Lineage Index

## 核心原则 (不可违反)

从 durable `descriptor` 构建父子关系索引，**不从事件顺序推断 ancestry**

## 三种 Scope

1. **self**: 仅自身事件
2. **directChildren**: 直接子级事件
3. **subtree**: 整棵子树事件

## 一致性保证

历史回放 (`/history`)、增量订阅 (`/events` SSE)、范围过滤 (scope query) 三条路径必须返回一致结果

## Legacy 处理

缺 `descriptor` → scope 过滤失败，不伪造 ancestry，UI 显示 "lineage unavailable"
