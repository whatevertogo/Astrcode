# Architecture 文档

## 目录说明

- [crates-dependency-graph.md](/D:/GitObjectsOwn/Astrcode/docs/architecture/crates-dependency-graph.md)：当前 workspace crate 依赖图，由脚本生成

## 推荐阅读顺序

1. 根目录 [PROJECT_ARCHITECTURE.md](/D:/GitObjectsOwn/Astrcode/PROJECT_ARCHITECTURE.md)
2. [crates-dependency-graph.md](/D:/GitObjectsOwn/Astrcode/docs/architecture/crates-dependency-graph.md)
3. 对应 ADR

## 用法建议

- 判断“现在代码的真实依赖关系”时，以依赖图和源码为准
- 判断“未来要收敛到什么结构”时，以 OpenSpec 变更文档为准
- 如果两者冲突，说明当前实现还没完成收敛，不能拿目标图替代现状图
