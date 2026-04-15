# Stabilize Conversation Surface

## Why

`release-terminal-astrcode` 已经把 conversation surface、client facade 和正式 CLI 路径打通，但代码结构里仍保留了明显的过渡痕迹：

- `terminal` 命名仍在部分内部实现中被当作主命名继续扩散
- CLI 状态最初以单一 `CliState` 聚合，容易继续膨胀
- `SessionRuntime` 仍以厚 façade 形式同时承担读写能力，`application` 端口也保留了偏宽的 escape hatch

这些问题不影响当前功能交付，但会继续推高后续维护成本，并削弱架构文档对实现的约束力。

## What Changes

- 坐实 `conversation` 作为正式读面命名，冻结 `terminal` 兼容 alias
- 将 CLI 状态模型收口为稳定子域，并减少 `AppController` 对平铺状态的直接依赖
- 在 `session-runtime` 内显式形成 `query` / `command` 边界，并收紧 `application` 侧端口
- 更新架构文档、验收脚本说明与相关 specs，使其与当前实现对齐

## Impact

- 不改变 HTTP/SSE wire shape
- 不新增产品能力
- 会带来局部内部命名和模块结构调整
- 后续 typed actor loop、进一步端口瘦身可以在此基础上继续推进
