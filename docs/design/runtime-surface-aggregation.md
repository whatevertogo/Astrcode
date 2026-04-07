# Runtime Surface 聚合对象设计

## 问题

当前 runtime surface 由多个并列 `Vec` 字段组成，带来三个问题：

1. 传参平铺，bootstrap / reload 链路冗长。
2. 新增一种贡献类型时，要同时改多个结构和函数签名。
3. “能力、技能、提示词、钩子”虽然都属于 surface，但语义层次不清楚。

## 设计结论

### 1. 用 `SurfaceLayer` 收口 runtime surface

新的聚合对象按语义分层，而不是继续堆平行字段：

- capabilities
- skills
- prompts
- hooks

### 2. 合并规则也按层定义

不同层的合并行为不同：

- invokers / handlers：追加
- registered names / seen block ids：集合并集
- catalog：保持单一稳定来源

这样可以把“如何合并”从调用点收回到对象本身。

### 3. `AssembledRuntimeSurface` 只组合结果，不再承载平铺细节

它应该持有：

- `router`
- `surface`
- plugin runtime 相关对象

而不是再次展开一组并列字段。

## 迁移原则

1. 先加 `SurfaceLayer`，不立即推翻旧结构。
2. 再逐步迁移 `assemble_runtime_surface` 与 `prepare_scoped_execution`。
3. 最后把 `RuntimeSurfaceContribution` 标记为 deprecated 并移除。

## 对应规范

- [../spec/runtime-surface-spec.md](../spec/runtime-surface-spec.md)
- [../spec/open-items.md](../spec/open-items.md)
