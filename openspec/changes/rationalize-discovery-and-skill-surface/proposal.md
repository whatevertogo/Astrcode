## Why

旧项目里存在独立的工具发现、模糊搜索、Skill Tool 等能力，但当前项目已经逐步建立：

- `kernel` capability surface
- capability semantic model
- skill catalog / materialization 链路

如果不先澄清“哪些发现能力仍然有产品价值”，迁移时很容易为了对齐旧接口重新引入重复表面与重复注册表，破坏当前架构。

因此这项迁移的目标不是“无脑恢复旧 discovery 模块”，而是先完成架构化收口：只保留真正有价值的发现能力，并让它们以现有 surface 为事实源。

## What Changes

- 重新评估旧项目中的工具搜索、Skill Tool、显式发现入口是否仍有产品价值。
- 若保留，则它们必须基于当前 capability surface / skill catalog 实现。
- 若不保留，则以明确 spec 形式宣布废弃，而不是让临时 skeleton 漂浮在仓库里。

## Capabilities

### New Capabilities

- `tool-and-skill-discovery`: 为工具与技能提供架构化的发现与查询能力。

### Modified Capabilities

- `capability-semantic-model`: 若发现能力需要语义搜索或展示字段，必须从现有能力语义模型扩展，而不是自建平行模型。

## Impact

- 工具发现与技能发现的事实源会统一收敛到当前 surface/catelog。
- 不再为了迁移而恢复旧 runtime 风格的独立注册表或空接口。
