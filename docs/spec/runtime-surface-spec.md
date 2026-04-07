# Runtime Surface 规范

## 1. 目标

把 runtime surface 从“并列字段拼接”收口为“按语义分层的聚合对象”，降低扩展成本并稳定组合规则。

## 2. 旧模型问题

旧的 `RuntimeSurfaceContribution` / `AssembledRuntimeSurface` 主要问题：

- 多个 `Vec` 并列传参
- 新增字段类型时修改面过大
- 组合规则散落在调用点
- 能力、技能、提示词、钩子的层次关系不清楚

## 3. 新模型

### 3.1 顶层对象

`SurfaceLayer` 是新的聚合根，至少包含四层：

| 层 | 作用 |
| --- | --- |
| `capabilities` | 工具/能力调用器 |
| `skills` | 技能与目录 |
| `prompts` | prompt 声明与 block 去重 |
| `hooks` | 生命周期钩子 |

### 3.2 子层字段要求

#### CapabilityLayer

- `invokers`
- `registered_names`

#### SkillLayer

- `specs`
- `catalog`

#### PromptLayer

- `declarations`
- `seen_block_ids`

#### HookLayer

- `handlers`

## 4. 合并语义

`SurfaceLayer` 的合并必须按层进行，而不是简单拼接整个对象。

| 层 | 合并规则 |
| --- | --- |
| capabilities | `invokers` 追加，`registered_names` 取并集 |
| skills | `specs` 追加，`catalog` 保留稳定单一来源 |
| prompts | `declarations` 追加，`seen_block_ids` 取并集 |
| hooks | `handlers` 追加 |

`SurfaceLayer::merge(...)` 与 `Add` 运算语义必须保持一致。

## 5. 与旧结构的兼容

### 5.1 转换规则

`RuntimeSurfaceContribution` 可以通过 `From` 转为 `SurfaceLayer`，用于迁移期兼容。

### 5.2 `AssembledRuntimeSurface`

新的 `AssembledRuntimeSurface` 应持有：

- `router`
- `surface`
- plugin runtime 相关对象

它不应继续暴露一组平铺的并列字段。

## 6. 迁移要求

迁移顺序应为：

1. 添加 `SurfaceLayer` 与各子层结构
2. 提供 `From<RuntimeSurfaceContribution>` 转换
3. 迁移 `assemble_runtime_surface`
4. 迁移 `prepare_scoped_execution`
5. 迁移其他调用方
6. 将 `RuntimeSurfaceContribution` 标记为 `#[deprecated]`
7. 后续版本移除旧结构

## 7. 非目标

- 不在当前阶段重新设计 router 本身
- 不在当前阶段改变 plugin runtime 生命周期
- 不为了分层而牺牲零拷贝和 `Arc` 传递模型

## 8. 对应文档

- 设计入口：[../design/runtime-surface-aggregation.md](../design/runtime-surface-aggregation.md)
- 开放项：[./open-items.md](./open-items.md)
