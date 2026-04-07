# Astrcode 文档目录

## 目录约定

- `adr/`：已经接受并生效的架构决策记录。
- `architecture/`：稳定的全局架构说明与路线图。
- `design/`：**短文档**，只表达问题、约束、方案与边界，尽量少写实现细节。
- `spec/`：**规范文档**，定义数据模型、协议、事件、状态机、API 与实现约束。
- `ideas/`：未定稿的想法与调研，不作为实现依据。

## 当前整理规则

这次整理把原来的：

- `docs/design/` 中的长篇设计稿
- `docs/plan/` 中的实施计划
- `docs/TODO/` 中的待办清单

重组为两层：

1. `docs/design/`：保留精简后的设计结论。
2. `docs/spec/`：承接规范、约束、开放项与后续待办。

## 推荐阅读顺序

1. `docs/architecture/README.md`
2. `docs/design/README.md`
3. `docs/spec/README.md`
4. 相关 ADR

## 说明

- 设计是否成立，以 `design/` 为入口。
- 实现怎么做、哪些字段和事件必须稳定，以 `spec/` 为准。
- 尚未完成但已经识别出的工作，统一收口到 `docs/spec/open-items.md`。
