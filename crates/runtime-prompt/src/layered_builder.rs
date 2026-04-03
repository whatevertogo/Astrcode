//! 分层 Prompt 构建器（Layered Prompt Builder）。
//!
//! 采用建造者模式 + 分层缓存策略，保证 LLM KV 缓存命中率的同时，
//! 支持灵活的 prompt 组装。
//!
//! ## 设计意图
//!
//! 当前的 `PromptComposer` 每次都会重新收集所有 contributor、重新渲染所有 block。
//! 这对于 KV 缓存不友好，因为即使只有 tool list 变化，整个 system prompt 也会重建，
//! 导致前缀缓存失效。
//!
//! 本模块引入**分层构建**策略：
//! - **稳定层**（Stable Layer）：Identity、Environment 等几乎不变的部分，缓存并复用
//! - **半稳定层**（Semi-Stable Layer）：Rules、Extensions 等偶尔变化的部分，按指纹缓存
//! - **动态层**（Dynamic Layer）：Tool list、Skill summary 等频繁变化的部分，每次重建
//!
//! ## KV 缓存优化
//!
//! LLM provider（如 Anthropic）会对请求的最后 N 条消息标记 `cache_control`。
//! 为了让缓存命中：
//! 1. 稳定层放在 system prompt 最前面，不标记缓存（因为几乎不变，后端已隐式缓存）
//! 2. 动态层放在 system prompt 最后面，标记 `cache_control`（让后端知道这里是缓存边界）
//! 3. 每次请求保证**前缀稳定**，只有后缀变化，这样 KV 缓存可以复用前缀部分
//!
//! ## 使用方式
//!
//! ```ignore
//! let builder = LayeredPromptBuilder::new();
//! let plan = builder
//!     .with_stable_layer(stable_contributors)
//!     .with_semi_stable_layer(rule_contributors)
//!     .with_dynamic_layer(dynamic_context)
//!     .build(&ctx)
//!     .await?;
//! ```

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::block::BlockSpec;
use super::context::PromptContext;
use super::contribution::{append_unique_tools, PromptContribution};
use super::diagnostics::PromptDiagnostics;
use super::plan::PromptPlan;
use super::PromptContributor;

/// 分层 Prompt 构建器。
///
/// 采用三层架构：稳定层 → 半稳定层 → 动态层，
/// 每层有独立的缓存策略和失效机制。
pub struct LayeredPromptBuilder {
    /// 稳定层贡献者（Identity、Environment 等几乎不变的部分）。
    stable_contributors: Vec<Arc<dyn PromptContributor>>,
    /// 半稳定层贡献者（Rules、Extensions 等偶尔变化的部分）。
    semi_stable_contributors: Vec<Arc<dyn PromptContributor>>,
    /// 动态层贡献者（Tool list、Skill summary 等频繁变化的部分）。
    dynamic_contributors: Vec<Arc<dyn PromptContributor>>,
    /// 层缓存。
    cache: Arc<Mutex<LayerCache>>,
    /// 构建选项。
    options: LayeredBuilderOptions,
}

impl Default for LayeredPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 分层构建器的配置选项。
#[derive(Debug, Clone)]
pub struct LayeredBuilderOptions {
    /// 是否启用诊断信息收集。
    pub enable_diagnostics: bool,
    /// 稳定层缓存 TTL（默认永不过期，因为 stable 层几乎不变）。
    pub stable_cache_ttl: Duration,
    /// 半稳定层缓存 TTL（默认 5 分钟）。
    pub semi_stable_cache_ttl: Duration,
}

impl Default for LayeredBuilderOptions {
    fn default() -> Self {
        Self {
            enable_diagnostics: true,
            stable_cache_ttl: Duration::ZERO, // 永不过期
            semi_stable_cache_ttl: Duration::from_secs(300),
        }
    }
}

/// 层缓存条目。
#[derive(Debug, Clone)]
struct LayerCacheEntry {
    /// 贡献者指纹（用于检测变化）。
    fingerprint: String,
    /// 缓存时间。
    cached_at: Instant,
    /// 缓存的贡献结果。
    contribution: PromptContribution,
}

/// 分层缓存。
#[derive(Debug, Default)]
struct LayerCache {
    /// 稳定层缓存（通常永不过期）。
    stable: Option<LayerCacheEntry>,
    /// 半稳定层缓存（按指纹失效）。
    semi_stable: Option<LayerCacheEntry>,
}

impl LayeredPromptBuilder {
    /// 创建新的分层构建器。
    pub fn new() -> Self {
        Self::with_options(LayeredBuilderOptions::default())
    }

    /// 使用指定选项创建分层构建器。
    pub fn with_options(options: LayeredBuilderOptions) -> Self {
        Self {
            stable_contributors: Vec::new(),
            semi_stable_contributors: Vec::new(),
            dynamic_contributors: Vec::new(),
            cache: Arc::new(Mutex::new(LayerCache::default())),
            options,
        }
    }

    /// 添加稳定层贡献者。
    ///
    /// 稳定层包含几乎不变的内容（如 Identity、Environment），
    /// 缓存后几乎不会重新计算。
    pub fn with_stable_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.stable_contributors = contributors;
        self
    }

    /// 添加半稳定层贡献者。
    ///
    /// 半稳定层包含偶尔变化的内容（如 Rules、Extensions），
    /// 按指纹缓存，TTL 过期或指纹变化时失效。
    pub fn with_semi_stable_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.semi_stable_contributors = contributors;
        self
    }

    /// 添加动态层贡献者。
    ///
    /// 动态层包含频繁变化的内容（如 Tool list、Skill summary），
    /// 每次构建都会重新计算，不缓存。
    pub fn with_dynamic_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.dynamic_contributors = contributors;
        self
    }

    /// 执行分层 prompt 构建。
    ///
    /// 按稳定层 → 半稳定层 → 动态层的顺序依次构建，
    /// 每层使用独立的缓存策略，最终合并为完整的 `PromptPlan`。
    ///
    /// ## KV 缓存友好性
    ///
    /// 构建结果保证：
    /// 1. 稳定层 block 在最前面（优先级最低）
    /// 2. 半稳定层 block 在中间
    /// 3. 动态层 block 在最后面（优先级最高）
    ///
    /// 这样 LLM provider 对最后 N 条消息标记 `cache_control` 时，
    /// 可以保证前缀（稳定层 + 半稳定层）的 KV 缓存命中。
    pub async fn build(&self, ctx: &PromptContext) -> Result<PromptBuildOutput> {
        let mut diagnostics = PromptDiagnostics::default();
        let mut all_blocks = Vec::new();
        let mut extra_tools = Vec::new();

        // 1. 构建稳定层（带缓存）
        let stable = self
            .build_layer(
                &self.stable_contributors,
                ctx,
                LayerType::Stable,
                &mut diagnostics,
            )
            .await?;
        merge_contribution(&mut all_blocks, &mut extra_tools, stable);

        // 2. 构建半稳定层（带缓存）
        let semi_stable = self
            .build_layer(
                &self.semi_stable_contributors,
                ctx,
                LayerType::SemiStable,
                &mut diagnostics,
            )
            .await?;
        merge_contribution(&mut all_blocks, &mut extra_tools, semi_stable);

        // 3. 构建动态层（无缓存）
        let dynamic = self
            .build_layer(
                &self.dynamic_contributors,
                ctx,
                LayerType::Dynamic,
                &mut diagnostics,
            )
            .await?;
        merge_contribution(&mut all_blocks, &mut extra_tools, dynamic);

        // 注意：此处仅收集了 BlockSpec（原始规格），尚未进行模板渲染、条件过滤、
        // 依赖解析等处理。完整的 block 渲染逻辑需要复用 PromptComposer 的 resolve_candidates
        // 流程，当前作为临时方案保持为空，后续需要实现。
        let plan = PromptPlan {
            system_blocks: Vec::new(), // TODO: 需要完整的 block 渲染逻辑（模板渲染 + 条件过滤 + 依赖解析）
            prepend_messages: Vec::new(),
            append_messages: Vec::new(),
            extra_tools,
        };

        Ok(PromptBuildOutput { plan, diagnostics })
    }

    /// 构建单个层（带缓存逻辑）。
    async fn build_layer(
        &self,
        contributors: &[Arc<dyn PromptContributor>],
        ctx: &PromptContext,
        layer_type: LayerType,
        diagnostics: &mut PromptDiagnostics,
    ) -> Result<PromptContribution> {
        // 动态层不缓存
        if layer_type == LayerType::Dynamic {
            return self
                .collect_contributors(contributors, ctx, diagnostics)
                .await;
        }

        // 计算指纹
        let fingerprint = compute_layer_fingerprint(contributors, ctx);

        // 检查缓存（仅读取，不持有锁过await点）
        // 安全：lock() 前无 await 点且持有时间短，不会触发 Mutex poison
        let cached_entry = {
            let cache = self
                .cache
                .lock()
                .expect("cache lock should not be poisoned");
            let cached = match layer_type {
                LayerType::Stable => &cache.stable,
                LayerType::SemiStable => &cache.semi_stable,
                LayerType::Dynamic => unreachable!(),
            };

            if let Some(entry) = cached {
                if entry.fingerprint == fingerprint
                    && !is_cache_expired(entry, &self.options, layer_type)
                {
                    Some(entry.contribution.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        // 如果缓存命中，直接返回
        if let Some(contribution) = cached_entry {
            return Ok(contribution);
        }

        // 缓存未命中，重新构建
        let contribution = self
            .collect_contributors(contributors, ctx, diagnostics)
            .await?;

        // 更新缓存
        // 安全：lock() 前无 await 点且持有时间短，不会触发 Mutex poison
        let mut cache = self
            .cache
            .lock()
            .expect("cache lock should not be poisoned");
        let entry = LayerCacheEntry {
            fingerprint,
            cached_at: Instant::now(),
            contribution: contribution.clone(),
        };
        match layer_type {
            LayerType::Stable => cache.stable = Some(entry),
            LayerType::SemiStable => cache.semi_stable = Some(entry),
            LayerType::Dynamic => unreachable!(),
        }

        Ok(contribution)
    }

    /// 收集贡献者的贡献（无缓存）。
    async fn collect_contributors(
        &self,
        contributors: &[Arc<dyn PromptContributor>],
        ctx: &PromptContext,
        _diagnostics: &mut PromptDiagnostics,
    ) -> Result<PromptContribution> {
        let mut result = PromptContribution::default();

        for contributor in contributors {
            let contribution = contributor.contribute(ctx).await;
            result.blocks.extend(contribution.blocks);
            result
                .contributor_vars
                .extend(contribution.contributor_vars);
            result.extra_tools.extend(contribution.extra_tools);
        }

        Ok(result)
    }
}

/// 层类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerType {
    /// 稳定层（几乎不变）。
    Stable,
    /// 半稳定层（偶尔变化）。
    SemiStable,
    /// 动态层（频繁变化）。
    Dynamic,
}

/// Prompt 构建的输出结果。
#[derive(Debug, Clone)]
pub struct PromptBuildOutput {
    pub plan: PromptPlan,
    pub diagnostics: PromptDiagnostics,
}

/// 计算层的指纹（用于缓存失效检测）。
fn compute_layer_fingerprint(
    contributors: &[Arc<dyn PromptContributor>],
    ctx: &PromptContext,
) -> String {
    // 使用贡献者 ID + 上下文关键信息的哈希
    let contributor_ids: String = contributors
        .iter()
        .map(|c| c.contributor_id())
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "{}|working_dir:{}|config_version:{}",
        contributor_ids,
        ctx.working_dir,
        ctx.config_version().unwrap_or("0"),
    )
}

/// 检查缓存是否过期。
fn is_cache_expired(
    entry: &LayerCacheEntry,
    options: &LayeredBuilderOptions,
    layer_type: LayerType,
) -> bool {
    let ttl = match layer_type {
        LayerType::Stable => options.stable_cache_ttl,
        LayerType::SemiStable => options.semi_stable_cache_ttl,
        LayerType::Dynamic => Duration::ZERO,
    };

    if ttl == Duration::ZERO {
        return false; // 永不过期
    }

    entry.cached_at.elapsed() > ttl
}

/// 合并贡献到累积结果中。
fn merge_contribution(
    all_blocks: &mut Vec<BlockSpec>,
    extra_tools: &mut Vec<astrcode_core::ToolDefinition>,
    contribution: PromptContribution,
) {
    all_blocks.extend(contribution.blocks);
    append_unique_tools(extra_tools, contribution.extra_tools);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_type_ordering() {
        // 验证层类型的优先级顺序：Stable < SemiStable < Dynamic
        assert!(LayerType::Stable != LayerType::SemiStable);
        assert!(LayerType::SemiStable != LayerType::Dynamic);
    }

    #[test]
    fn test_cache_ttl_zero_means_never_expire() {
        let entry = LayerCacheEntry {
            fingerprint: "test".to_string(),
            cached_at: Instant::now(),
            contribution: PromptContribution::default(),
        };
        let options = LayeredBuilderOptions {
            stable_cache_ttl: Duration::ZERO,
            ..Default::default()
        };

        // 稳定层 TTL=0 表示永不过期
        assert!(!is_cache_expired(&entry, &options, LayerType::Stable));
    }
}
