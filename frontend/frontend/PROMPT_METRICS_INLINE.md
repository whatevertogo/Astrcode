# Prompt Metrics 内联显示改进

## 改动概述

将 Prompt 指标从独立的卡片消息改为在 Assistant 消息底部的内联紧凑显示。

## 修改的文件

### 1. `src/components/Chat/AssistantMessage.tsx`
- 添加 `metrics?: PromptMetricsMessage` 可选属性
- 添加 `formatTokenCount` 函数，将大于 1000 的数字格式化为 "k" 单位
- 在消息内容底部添加 metrics 显示（仅在非流式状态下显示）

### 2. `src/components/Chat/AssistantMessage.module.css`
- 添加 `.metricsInline` 样式类
- 使用灰色小字、顶部边框分隔
- 字体大小 12px，颜色 #6b7280

### 3. `src/components/Chat/MessageList.tsx`
- 修改 `renderMessageContent` 接受 `metrics` 参数
- 修改 `renderMessageRow` 接受 `nextMessage` 参数
- 修改 `renderThreadItems` 传递 `nextMessage`
- 当检测到 `assistant` 消息后跟 `promptMetrics` 消息时，将 metrics 附加到 assistant 消息
- `promptMetrics` 消息不再单独渲染（返回 null）

## 显示格式

```
📊 8k tokens · 108k/128k context · Cache: —/—
```

### 格式说明
- 📊 图标开头
- `estimatedTokens` - 估算的 token 数
- `effectiveWindow/contextWindow` - 有效窗口/上下文窗口
- `Cache: cacheReadInputTokens/cacheCreationInputTokens` - 缓存读取/写入

### 数字格式化
- 小于 1000：显示原始数字（如 "856"）
- 大于等于 1000：显示为 k 单位（如 "8k", "108k"）
- undefined：显示为 "—"

## 用户体验改进

### 之前
- Prompt 指标显示为独立的卡片消息
- 占用较多垂直空间
- 视觉上与对话内容分离

### 之后
- Metrics 直接附加在对应的 assistant 消息底部
- 紧凑的单行显示
- 不打断阅读流
- 只在消息完成后显示（流式状态下不显示）

## 技术细节

### 消息关联逻辑
在 `renderThreadItems` 中：
1. 遍历消息列表
2. 对于每个 `assistant` 消息，检查下一条消息是否为 `promptMetrics`
3. 如果是，将 `promptMetrics` 作为 `metrics` 属性传递给 `AssistantMessage`
4. `promptMetrics` 消息本身返回 null，不再单独渲染

### 样式设计
- 顶部边框：`1px solid rgba(0, 0, 0, 0.06)` - 轻微分隔
- 上边距：`12px` - 与内容保持适当距离
- 上内边距：`8px` - 边框与文字的间距
- 字体大小：`12px` - 小字显示，不抢眼
- 颜色：`#6b7280` - 中性灰色，次要信息

## 兼容性

- 保留了 `PromptMetricsMessage` 组件（未删除），以防需要回退
- 如果 `promptMetrics` 消息没有对应的 `assistant` 消息，不会崩溃（只是不显示）
- 向后兼容现有的消息结构
