## Context

“完全迁移”不等于“把旧项目每个接口和每个模块都原样复活”。对于 discovery 和 skill surface，这个问题尤其明显：旧项目里可能同时存在显式 API、模糊搜索、Skill Tool、内部目录结构等多套入口。

当前项目已经有更好的目标态：

- `kernel` capability surface 作为统一能力事实源
- capability semantic model 作为能力语义描述
- skill catalog / materializer 作为技能资产事实源

因此本 change 必须先完成“保留/删除”的架构判断，再进入实现。

## Design Decisions

### 1. 发现能力必须依赖统一事实源

若保留工具发现或技能发现能力，它们的数据来源只能是：

- capability surface
- capability semantic model
- skill catalog

不允许再创建独立 runtime registry 或独立 discovery cache 作为第二事实源。

### 2. Skill Tool 只有在确有产品价值时才保留

若现有 prompt / capability routing / skill catalog 已足够表达技能能力，则不强制恢复单独 Skill Tool。只有满足真实产品场景时才保留，并且其实现仍必须依赖现有 catalog。

### 3. 允许“明确废弃”，不允许“挂空壳”

如果某个旧发现入口已不再需要，则应通过 spec 明确废弃并从实现层删除，不允许留下空路由、空 façade 或空 manager。

### 4. capability semantic model 是扩展点，不是旁路

若需要支持模糊搜索、展示标签、调用建议、可见性排序等能力，优先扩展现有 capability semantic model，而不是新增平行的搜索模型。

## Risks and Mitigations

### 风险：为了追平旧项目而恢复多套事实源

缓解：

- 明确 capability surface / skill catalog 是唯一事实源。

### 风险：过早删除仍有产品价值的 discovery 能力

缓解：

- 先做旧功能盘点与使用面判断，再决定保留或废弃。
