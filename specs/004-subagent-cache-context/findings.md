# Findings: 子智能体会话与缓存边界优化

本文件记录 004 实现落地后，当前仓库里已经成立、且会继续约束后续演进的事实。

## Finding 1: `IndependentSession` 已完成 cutover，shared history 只剩 legacy 读侧语义

- 新建 child 已默认进入独立 `child_session_id` durable 历史。
- `runtime-session` / `server` 对 legacy shared-history 样本统一返回 `unsupported_legacy_shared_history`，不再进入读取、回放或恢复兼容逻辑。
- `SharedSession` 仍出现在部分测试夹具、协议 DTO 和前端读模型中，但这些值只用于 legacy 样本识别与错误展示，不再代表支持中的新写入路径。

**Implication**  
后续如果看到 `SharedSession`，应优先把它理解成“旧数据形态”或“测试夹具输入”，而不是可继续扩展的新功能面。

## Finding 2: resume 现在以 child session durable replay 为真相

- resume 会先定位 `ChildSessionNode`，再基于 child session durable 历史恢复可见状态。
- 恢复成功时沿用原 `child_session_id`，同时 mint 新的 `sub_run_id` / execution instance。
- 谱系不一致、缺失 child session、损坏历史等情况都会显式失败并产出父侧可见错误。

**Implication**  
后续 resume 相关改动必须继续围绕 durable replay 与 lineage 校验展开，不能再退回“从空状态重开一轮”的语义。

## Finding 3: 父传子的背景已经改为 prompt 层结构化继承

- child 首条任务消息现在只承载 `task_payload`。
- 父 compact summary 与 recent tail 已进入 `PromptDeclaration` 的 `Inherited` 层，不再落入 durable `UserMessage`。
- recent tail 已带有确定性裁剪、工具输出短摘要和预算裁边。

**Implication**  
任何未来新增的父传子背景，都应优先进入 inherited blocks，而不是回退到拼接任务文本。

## Finding 4: 父唤醒已经切到 runtime-only 交付桥接

- child terminal delivery 会先落父侧边界事实，再进入 `runtime-agent-control` 的内存队列。
- 父空闲时通过运行时 wake turn 消费一次性交付声明，不再写 durable `ReactivationPrompt`。
- durable 历史里保留的是 started / resumed / delivered / failed / closed 等边界事实，不再混入机制性 `UserMessage`。

**Implication**  
“父可追溯”与“父可继续处理”已经被拆成 durable facts 与 runtime bridge 两层，不应再把两者重新耦合成一条 durable 消息链。

## Finding 5: `runtime-prompt` 指纹与 inherited cache boundary 已真正接管复用判定

- `Inherited` 层已经拥有独立缓存段，`compact_summary` 与 `recent_tail` 各自形成失效边界。
- 缓存命中/失效不再由 `runtime-execution` 手写 key，而是依赖 `runtime-prompt` 的 fingerprint 与 LayerCache。
- provider cache telemetry 已贯通到 prompt metrics 和执行观测。

**Implication**  
后续如果要调整缓存命中率，应优先从 prompt 输入和 fingerprint 维度诊断，而不是在 execution 层追加旁路哈希。

## Finding 6: 父侧交付缓冲仍然是进程内能力，但 durable 可追溯面已经稳定

- `runtime-agent-control` 的父交付队列仍是内存态，提供 FIFO、去重和繁忙时重排。
- 进程重启后缓冲不会保留，但父侧 `ChildSessionNotification` 与 child session 入口仍可重建用户可见追溯面。

**Implication**  
当前实现承诺的是“进程存活期间可靠桥接 + durable 事实可追溯”，不是跨重启消息队列。

## Finding 7: mixed-thread 视图不再是父视图主语，但仍保留为子线程浏览/调试读模型

- 默认父视图已经转向 child summary card + open-session link。
- 前端的 mixed-thread helper 仍在子线程详情浏览、路径导航和测试夹具中被使用。
- 这条 helper 现在是补充视图，不再代表父 durable 历史的主真相。

**Implication**  
后续若继续精简前端，应先区分“默认父摘要投影”和“子线程浏览辅助树”，不能把仍在真实 UI 路径里消费的 helper 误判成死代码。

## Public-Surface Removal Notes

以下 surface 已在 004 cutover 后收紧；后续实现与测试必须继续按“直接移除旧语义”维护：

- `runtime-session` 不再支持共享写入历史的读取、回放与恢复；legacy 数据统一返回稳定错误。
- 父会话 durable 历史里不再允许写入 `ReactivationPrompt` 或其他机制性 `UserMessage`。
- child 首条任务消息不再承载父 compact summary / recent tail；这些内容只存在于 inherited prompt blocks。
- 父默认视图不再依赖 mixed subrun thread tree 反推 child 内部过程，而是消费 child summary / open-session link。
- `IndependentSession` 已不再是实验能力；任何仍假设它需要 feature gate 的调用面都应视为遗留实现。
