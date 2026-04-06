自动 Compact 最终方案摘要
核心原则

仍然保留 70% 作为自动 compact 启动阈值。
但 70% 的含义不是“立刻替换上下文”，而是：

到 70% 时开始自动准备 compact
自动寻找合适关键点再应用 compact
关键点不到，不切
超过保底阈值时，下一个关键点必须切

这样既保留你要的“提前无感准备”，也避免“中途换脑子”的断层风险。

你真正要实现的需求
1. 70% 触发自动预备 compact

当上下文达到 70%：

runtime 标记 compact_pending = true
可后台 fork 一个 compact agent
开始为旧历史生成 compact note / handoff note
此时不替换主上下文

也就是：

70% = 开始预热，不是开始切换。
这保留了你最初“自动、无感、提前准备”的体验目标。

2. 自动寻找“合适关键点”再应用

只有遇到合适关键点，才真正应用 compact。

合适关键点定义

我建议你文档里直接写成：

当前没有 pending tool call
当前没有 pending subagent
当前没有流式输出
assistant final 已经结束
loop 进入 AwaitUserInput / IdleSafePoint

也就是：

agent loop 停稳点，不是模型“主观觉得完成了”。
因为你前面也已经意识到，真正危险的是“当前 turn 完成就替换”。

3. 自动 compact 只压旧历史，不动最近工作集

compact 后的新上下文不是简单：

摘要 + 后 30%

而应该是：

system / rules
compact boundary note
handoff note
recent working set
最近几轮原始对话
当前新输入

因为你们前面的讨论已经指出：
不能粗暴删前 70%，不然会“瞬间失忆”。

4. compact 产物不是 JSON，而是高密度自然语言笔记

这点你已经想清楚了，应该固定下来。

产物形式

不要：

State_Snapshot.json
纯结构化 JSON 摘要

要：

Handoff Note
高密度自然语言交接单
保留文件指针、关键决策、纠错、未解问题

因为 JSON 太硬，语义延续感差；
而自然语言交接单更适合对话续接。

5. fork agent 可以写摘要，但必须有强约束模板

你前面也已经确认：

可以直接让 fork agent 去写摘要和笔记。
但不能只给一句“帮我总结一下”，否则一定会写成流水账。

必须写入的内容

你的 compact prompt 至少要强制包含：

当前目标
已做决定
改过哪些文件 / 函数 / 行号指针
用户纠正过什么
约束 / 禁忌
当前卡点
未完成事项
明确禁止
不准贴大段代码
不准写流水账
不准写空洞总结句
不准丢文件指针
6. 应用 compact 时必须插入边界声明

真正应用 compact 时，要在 system/rules 后插一个边界说明：

早期对话已被整理为下方交接笔记；
后续优先参考最近原始消息；
如果笔记缺细节，应重新读文件或询问，而不是猜。

这是为了避免模型偷偷把“历史摘要”当成“完整原文”。
你们前面的讨论里，这个点就是为了解决无感切换导致的断层。

7. 手动 compact 和保底 compact 仍然保留

你的系统最后应该有三种 compact：

自动 compact
70% 开始准备
到 safe point 自动应用
手动 compact
用户随时点
若 loop 正在运行，则排队到下一个 safe point 执行
保底强制 compact
如果已经接近极限
则下一个 safe point 必须 compact
防止上下文直接爆掉
推荐状态机
enum LoopState {
    RunningModel,
    RunningTool,
    RunningSubAgent,
    Streaming,
    AwaitUserInput,
}

enum CompactState {
    None,
    PendingAuto,      // 70% 触发
    CandidateReady,   // 笔记已生成
    PendingManual,
    RequiredHard,     // 保底强制
    Applying,
    Failed,
}
推荐执行流
A. 到 70%
标记 PendingAuto
后台 fork compact agent
生成 handoff note
存为 CandidateReady
B. loop 继续跑
不替换
不打断
用户无感
C. 到合适关键点

如果：

CandidateReady
且 LoopState == AwaitUserInput

则：

删除旧历史
注入 boundary note
注入 handoff note
保留 recent working set
完成 compact
D. 下一轮继续

模型看到的是：

规则层
compact 边界说明
交接笔记
最近原始上下文
当前用户输入
文档里可以直接写成这句话
设计定义

系统在上下文使用率达到 70% 时启动自动 compact 预备流程。
该流程不会立即替换当前上下文，而是先后台生成交接笔记。
只有当 agent loop 进入安全停稳点（safe point）时，系统才会自动应用 compact。
若未到安全点，则继续等待；若达到保底阈值，则在下一个安全点强制执行 compact。

你的文档结论版
目标
保持无感体验
提前准备 compact
避免中途切换导致失忆
用自然语言交接笔记承接旧历史
关键策略
70% 启动
关键点应用
旧历史压缩
最近工作集保留
边界声明注入
手动与保底机制并存
本质

这不是“70% 直接裁历史”，而是：

70% 开始找机会，在合适关键点自动 compact。