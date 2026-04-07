# Astrcode Architecture

## 这组文档讲什么

`architecture/` 只回答稳定架构问题：

- 仓库按什么层次划分
- crate 之间如何依赖
- 前端、技能系统和 runtime 各自承担什么责任
- 未来演进的大方向是什么

实现字段、事件协议和开放项已经转移到：

- `D:\GitObjectsOwn\Astrcode\docs\design`
- `D:\GitObjectsOwn\Astrcode\docs\spec`

## 推荐阅读顺序

1. [architecture.md](./architecture.md) — 系统总览与分层边界
2. [crates-dependency-graph.md](./crates-dependency-graph.md) — crate 依赖附录（自动生成）
3. [frontend-architecture.md](./frontend-architecture.md) — 前端状态、数据流与边界
4. [skills-architecture.md](./skills-architecture.md) — Skill 系统架构
5. [agent-loop-roadmap.md](./agent-loop-roadmap.md) — Agent Runtime 演进路线

## 相关文档

- [../README.md](../README.md) — 文档总索引
- [../design/README.md](../design/README.md) — 精简设计文档
- [../spec/README.md](../spec/README.md) — 规范文档
- [../adr/](../adr/) — 已接受 ADR

## 说明

- `crates-dependency-graph.md` 是自动生成文件，主要当作附录看。
- 如果设计结论与规范细节冲突，以 `spec/` 为准；如果实现与设计冲突，以代码和 ADR 为准，再回写文档。
