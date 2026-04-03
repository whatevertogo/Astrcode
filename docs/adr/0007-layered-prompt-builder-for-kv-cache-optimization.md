# ADR-0007: 分层 Prompt 构建器（设计决策）

**状态**: 设计完成，代码已实现 (`crates/runtime-prompt/src/layered_builder.rs`)，**未投入生产使用**

## 当前状态

`LayeredPromptBuilder` 已在 `runtime-prompt` crate 中实现，但**未被 `agent_loop` 或 `PromptComposer` 调用**。  
当前生产代码使用 `PromptComposer` 完成 prompt 组装（贡献者级缓存）。

`LayeredPromptBuilder` 的 `build()` 方法存在 TODO：`system_blocks` 为空（全量 block 渲染、模板、条件分支、依赖解析尚未完成），目前仅收集 `extra_tools`。

## 建议

- **保留设计文档**：KV cache 优化思路值得参考
- **标记实现状态**：代码中标注 `#[allow(dead_code)]` 或添加 `TODO` 注释
- **后续激活**：如果需要此优化，需要在 `prompt_runtime` 中接入 `LayeredPromptBuilder` 替换 `PromptComposer`

---

# 分层 Prompt 构建器使用指南

## 问题背景

当前的 `PromptComposer` 每次都会重新收集所有 contributor、重新渲染所有 block。这对于 LLM 的 KV 缓存不友好：

- Anthropic/OpenAI 等 provider 会对请求的**最后 N 条消息**标记 `cache_control`
- 如果整个 system prompt 每次都重建，即使只有 tool list 变化，前缀缓存也会失效
- 这导致每次请求都要重新计算 KV cache，增加延迟和成本

## 解决方案

`LayeredPromptBuilder` 采用**三层架构**：

```
┌─────────────────────────────────────────┐
│  稳定层 (Stable Layer)                   │  ← 几乎不变，永久缓存
│  - Identity (AI 身份)                    │
│  - Environment (工作环境)                │
├─────────────────────────────────────────┤
│  半稳定层 (Semi-Stable Layer)            │  ← 偶尔变化，按指纹缓存
│  - User Rules (~/.astrcode/AGENTS.md)   │
│  - Project Rules (./AGENTS.md)          │
│  - Extension Instructions              │
├─────────────────────────────────────────┤
│  动态层 (Dynamic Layer)                  │  ← 频繁变化，每次重建
│  - Tool List                            │
│  - Skill Summary                        │
│  - Workflow Examples                    │
└─────────────────────────────────────────┘
         ↓ LLM KV Cache Boundary ↓
    (cache_control 标记在此处)
```

## 使用示例

```rust
use astrcode_runtime_prompt::{
    LayeredPromptBuilder, LayeredBuilderOptions, PromptContext,
    contributors::{IdentityContributor, EnvironmentContributor},
};

// 1. 创建分层构建器
let builder = LayeredPromptBuilder::new()
    // 稳定层：Identity + Environment（几乎不变）
    .with_stable_layer(vec![
        Arc::new(IdentityContributor),
        Arc::new(EnvironmentContributor),
    ])
    // 半稳定层：Rules（文件变化时失效）
    .with_semi_stable_layer(vec![
        Arc::new(AgentsMdContributor),
    ])
    // 动态层：Tool list + Skill summary（每次 turn 可能变化）
    .with_dynamic_layer(vec![
        Arc::new(CapabilityPromptContributor),
        Arc::new(SkillSummaryContributor),
    ]);

// 2. 构建 prompt
let ctx = PromptContext {
    working_dir: "/workspace/my-project".to_string(),
    tool_names: vec!["shell".to_string(), "readFile".to_string()],
    // ... 其他字段
};

let output = builder.build(&ctx).await?;
let system_prompt = output.plan.render_system().unwrap();
```

## KV 缓存优化原理

### Anthropic Prompt Caching

Anthropic API 支持对消息标记 `cache_control`：

```rust
// 在 anthropic.rs 中，对最后 2 条消息启用缓存
let last_messages = &messages[messages.len().saturating_sub(2)..];
for msg in last_messages {
    msg.cache_control = Some(CacheControl { type: "ephemeral" });
}
```

### 分层构建如何保证缓存命中

1. **稳定层**放在 system prompt 最前面
   - 几乎不变，LLM 后端会隐式缓存
   - 不需要显式标记 `cache_control`

2. **动态层**放在 system prompt 最后面
   - 频繁变化（如 tool list）
   - LLM provider 对这部分标记 `cache_control`
   - 即使这部分缓存失效，**前缀（稳定层 + 半稳定层）仍然命中**

3. **前缀稳定性**是关键
   - 只要稳定层和半稳定层不变，KV cache 的前缀部分就可以复用
   - 只有动态层需要重新计算，大幅降低延迟

## 缓存策略配置

```rust
let options = LayeredBuilderOptions {
    enable_diagnostics: true,
    stable_cache_ttl: Duration::ZERO,        // 永不过期
    semi_stable_cache_ttl: Duration::from_secs(300), // 5 分钟
};

let builder = LayeredPromptBuilder::with_options(options);
```

## 与现有 PromptComposer 的关系

`LayeredPromptBuilder` 是 `PromptComposer` 的补充，不是替代：

| 特性 | PromptComposer | LayeredPromptBuilder |
|------|----------------|----------------------|
| 缓存粒度 | Contributor 级别 | 层级别 |
| KV 缓存友好性 | 一般（全量重建） | 理论优秀（前缀稳定） |
| 适用场景 | 生产环境 | 设计阶段（未接入） |
| 复杂度 | 低 | 中 |
| 实现状态 | ✅ 生产使用 | ⚠️ 未接入 |

> **注意**：当前运行时使用 `PromptComposer`，`LayeredPromptBuilder` 未启用。

## 下一步优化建议

1. **增量重压缩**：当前 `auto_compact` 总是重新摘要完整前缀，可升级为增量重压缩
2. **多模态支持**：压缩前剥离/降采样图片
3. **Claude 风格部分压缩**：支持 "from" 方向的部分压缩，保留更多上下文
4. **配置版本追踪**：当 AGENTS.md 等文件变化时，自动递增 `config_version`
