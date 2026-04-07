//! 分层 Prompt 构建器（Layered Prompt Builder）。
//!
//! 采用“按层独立 build，再合并最终 plan”的方式，把稳定前缀明确沉淀到
//! `PromptPlan.system_blocks` 的层级元数据中，供 Anthropic prompt caching 使用。

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;

use super::{
    PromptBuildOutput, PromptComposer, PromptComposerOptions, PromptContext, PromptContributor,
    PromptDiagnostics, PromptLayer, PromptPlan, ValidationLevel,
};

/// 分层 Prompt 构建器。
///
/// 采用三层架构：稳定层 → 半稳定层 → 动态层。
/// 每层单独执行完整的 `PromptComposer` 管线，再按层级合并结果。
pub struct LayeredPromptBuilder {
    stable_contributors: Vec<Arc<dyn PromptContributor>>,
    semi_stable_contributors: Vec<Arc<dyn PromptContributor>>,
    dynamic_contributors: Vec<Arc<dyn PromptContributor>>,
    cache: Arc<Mutex<LayerCache>>,
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
    /// 渲染/验证失败时的处理级别。
    pub validation_level: ValidationLevel,
}

impl Default for LayeredBuilderOptions {
    fn default() -> Self {
        Self {
            enable_diagnostics: true,
            stable_cache_ttl: Duration::ZERO,
            semi_stable_cache_ttl: Duration::from_secs(300),
            validation_level: ValidationLevel::Warn,
        }
    }
}

#[derive(Debug, Clone)]
struct LayerCacheEntry {
    fingerprint: String,
    cached_at: Instant,
    output: PromptBuildOutput,
}

#[derive(Debug, Default)]
struct LayerCache {
    stable: Option<LayerCacheEntry>,
    semi_stable: Option<LayerCacheEntry>,
}

impl LayeredPromptBuilder {
    pub fn new() -> Self {
        Self::with_options(LayeredBuilderOptions::default())
    }

    pub fn with_options(options: LayeredBuilderOptions) -> Self {
        Self {
            stable_contributors: Vec::new(),
            semi_stable_contributors: Vec::new(),
            dynamic_contributors: Vec::new(),
            cache: Arc::new(Mutex::new(LayerCache::default())),
            options,
        }
    }

    pub fn with_stable_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.stable_contributors = contributors;
        self
    }

    pub fn with_semi_stable_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.semi_stable_contributors = contributors;
        self
    }

    pub fn with_dynamic_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
        self.dynamic_contributors = contributors;
        self
    }

    /// 执行分层 prompt 构建。
    ///
    /// 每层都会运行完整的 composer 流程，因此不会再丢失模板渲染、
    /// 条件过滤、依赖解析和诊断。
    pub async fn build(&self, ctx: &PromptContext) -> Result<PromptBuildOutput> {
        let mut diagnostics = PromptDiagnostics::default();
        let mut plan = PromptPlan::default();

        for (layer_type, contributors) in [
            (LayerType::Stable, &self.stable_contributors),
            (LayerType::SemiStable, &self.semi_stable_contributors),
            (LayerType::Dynamic, &self.dynamic_contributors),
        ] {
            let output = self.build_layer(contributors, ctx, layer_type).await?;
            diagnostics.items.extend(output.diagnostics.items);
            plan.extend_with_layer(output.plan, layer_type.prompt_layer());
        }

        Ok(PromptBuildOutput { plan, diagnostics })
    }

    async fn build_layer(
        &self,
        contributors: &[Arc<dyn PromptContributor>],
        ctx: &PromptContext,
        layer_type: LayerType,
    ) -> Result<PromptBuildOutput> {
        if contributors.is_empty() {
            return Ok(PromptBuildOutput {
                plan: PromptPlan::default(),
                diagnostics: PromptDiagnostics::default(),
            });
        }

        if layer_type == LayerType::Dynamic {
            return self.render_layer(contributors, ctx).await;
        }

        let fingerprint = compute_layer_fingerprint(contributors, ctx);
        if let Some(output) = self.lookup_cache(layer_type, &fingerprint) {
            return Ok(output);
        }

        let output = self.render_layer(contributors, ctx).await?;
        self.store_cache(layer_type, fingerprint, output.clone());
        Ok(output)
    }

    async fn render_layer(
        &self,
        contributors: &[Arc<dyn PromptContributor>],
        ctx: &PromptContext,
    ) -> Result<PromptBuildOutput> {
        let mut composer = PromptComposer::new(PromptComposerOptions {
            enable_diagnostics: self.options.enable_diagnostics,
            validation_level: self.options.validation_level,
            // 分层 build 会重建临时 composer，因此这里不再依赖 contributor 级 TTL；
            // 由 `LayeredPromptBuilder` 自己承接跨 step 的层缓存。
            cache_ttl: Duration::ZERO,
        });
        for contributor in contributors {
            composer = composer.with_contributor(Arc::clone(contributor));
        }
        composer.build(ctx).await
    }

    fn lookup_cache(&self, layer_type: LayerType, fingerprint: &str) -> Option<PromptBuildOutput> {
        let cache = self
            .cache
            .lock()
            .expect("layer cache lock should not be poisoned");
        let entry = match layer_type {
            LayerType::Stable => cache.stable.as_ref(),
            LayerType::SemiStable => cache.semi_stable.as_ref(),
            LayerType::Dynamic => None,
        }?;

        if entry.fingerprint == fingerprint && !is_cache_expired(entry, &self.options, layer_type) {
            Some(entry.output.clone())
        } else {
            None
        }
    }

    fn store_cache(&self, layer_type: LayerType, fingerprint: String, output: PromptBuildOutput) {
        let mut cache = self
            .cache
            .lock()
            .expect("layer cache lock should not be poisoned");
        let entry = LayerCacheEntry {
            fingerprint,
            cached_at: Instant::now(),
            output,
        };

        match layer_type {
            LayerType::Stable => cache.stable = Some(entry),
            LayerType::SemiStable => cache.semi_stable = Some(entry),
            LayerType::Dynamic => {},
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerType {
    Stable,
    SemiStable,
    Dynamic,
}

impl LayerType {
    fn prompt_layer(self) -> PromptLayer {
        match self {
            Self::Stable => PromptLayer::Stable,
            Self::SemiStable => PromptLayer::SemiStable,
            Self::Dynamic => PromptLayer::Dynamic,
        }
    }
}

fn compute_layer_fingerprint(
    contributors: &[Arc<dyn PromptContributor>],
    ctx: &PromptContext,
) -> String {
    contributors
        .iter()
        .map(|contributor| {
            format!(
                "{}:{}:{}",
                contributor.contributor_id(),
                contributor.cache_version(),
                contributor.cache_fingerprint(ctx)
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

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

    if ttl.is_zero() {
        return false;
    }

    entry.cached_at.elapsed() > ttl
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;

    use super::*;
    use crate::{BlockKind, BlockSpec, PromptContribution};

    struct StaticContributor {
        id: &'static str,
        block_id: &'static str,
        title: &'static str,
        content: &'static str,
    }

    #[async_trait]
    impl PromptContributor for StaticContributor {
        fn contributor_id(&self) -> &'static str {
            self.id
        }

        async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
            PromptContribution {
                blocks: vec![BlockSpec::system_text(
                    self.block_id,
                    BlockKind::ExtensionInstruction,
                    self.title,
                    self.content,
                )],
                ..PromptContribution::default()
            }
        }
    }

    fn test_context() -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: Vec::new(),
            capability_descriptors: Vec::new(),
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn layered_builder_merges_non_empty_plan_and_marks_layers() {
        let builder = LayeredPromptBuilder::new()
            .with_stable_layer(vec![Arc::new(StaticContributor {
                id: "stable",
                block_id: "stable-block",
                title: "Stable",
                content: "stable content",
            })])
            .with_semi_stable_layer(vec![Arc::new(StaticContributor {
                id: "semi",
                block_id: "semi-block",
                title: "Semi",
                content: "semi content",
            })])
            .with_dynamic_layer(vec![Arc::new(StaticContributor {
                id: "dynamic",
                block_id: "dynamic-block",
                title: "Dynamic",
                content: "dynamic content",
            })]);

        let output = builder
            .build(&test_context())
            .await
            .expect("layered build should succeed");

        assert_eq!(output.plan.system_blocks.len(), 3);
        assert_eq!(
            output
                .plan
                .ordered_system_blocks()
                .into_iter()
                .map(|block| block.layer)
                .collect::<Vec<_>>(),
            vec![
                PromptLayer::Stable,
                PromptLayer::SemiStable,
                PromptLayer::Dynamic
            ]
        );
    }

    #[test]
    fn stable_zero_ttl_never_expires() {
        let entry = LayerCacheEntry {
            fingerprint: "fp".to_string(),
            cached_at: Instant::now(),
            output: PromptBuildOutput {
                plan: PromptPlan::default(),
                diagnostics: PromptDiagnostics::default(),
            },
        };
        let options = LayeredBuilderOptions {
            stable_cache_ttl: Duration::ZERO,
            ..Default::default()
        };

        assert!(!is_cache_expired(&entry, &options, LayerType::Stable));
    }
}
