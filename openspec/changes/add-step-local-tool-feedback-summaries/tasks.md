> 说明：在重新对照 Claude Code 的 persisted-output / reread 主路径后，本 change 已被
> `stabilize-persisted-tool-result-references` 取代，不再作为独立实现目标继续推进。
> 这样可以避免在 Astrcode 中同时维护两条 prompt 反馈主路径。

## 1. 方案收敛

- [x] 1.1 确认不再在 `crates/session-runtime/src/turn/` 中引入独立的 step-local feedback package 协议
- [x] 1.2 将“工具大结果如何进入下一轮 prompt”的主实现迁移到 `stabilize-persisted-tool-result-references`
- [x] 1.3 保留该 change 仅作为被 supersede 的历史设计记录，不进入代码实现

## 2. 验证

- [x] 2.1 确认 request 层当前未引入新的 feedback package / summary 主路径
- [x] 2.2 确认 persisted-output + aggregate budget 已覆盖原 change 想解决的主问题
