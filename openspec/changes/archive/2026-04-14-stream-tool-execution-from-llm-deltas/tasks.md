## 1. 流式工具候选组装与调度骨架

- [x] 1.1 调整 `crates/session-runtime/src/turn/llm_cycle.rs`，把 `ToolCallDelta` 从“仅 live 信号”升级为可供 runner 消费的流式工具片段输入
- [x] 1.2 在 `crates/session-runtime/src/turn/runner/step.rs` 增加 step-local 的 tool call assembler + streaming planner / scheduler
- [x] 1.3 明确只对白名单场景开放提前执行：参数完整、identity 稳定且 `concurrency_safe` 的工具调用

## 2. 提前执行与有序落盘

- [x] 2.1 调整 `crates/session-runtime/src/turn/tool_cycle.rs`，支持提前执行结果的 step-local 收集与 assistant 定稿后的统一 durable flush
- [x] 2.2 保持副作用工具与未闭合参数调用走现有保守路径，避免 speculative 执行
- [x] 2.3 为提前执行与 discard / fallback / 取消路径补充稳定的日志、指标与收口处理

## 3. 验证

- [x] 3.1 为 `llm_cycle`、`runner/step`、`tool_cycle` 增加流式 tool 调度测试
- [x] 3.2 运行 `cargo test -p astrcode-session-runtime`
- [x] 3.3 运行 `cargo clippy -p astrcode-session-runtime --all-targets --all-features -- -D warnings`
