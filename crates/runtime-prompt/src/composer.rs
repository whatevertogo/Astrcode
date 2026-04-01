use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use astrcode_core::LlmMessage;

use super::contributors::{
    AgentsMdContributor, EnvironmentContributor, IdentityContributor, SkillSummaryContributor,
};
use super::diagnostics::{DiagnosticLevel, DiagnosticReason, PromptDiagnostic, PromptDiagnostics};
use super::{
    append_unique_tools, BlockCondition, BlockContent, BlockKind, BlockSpec, PromptBlock,
    PromptContext, PromptContribution, PromptContributor, PromptPlan, RenderTarget,
    TemplateRenderError, ValidationPolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationLevel {
    Off,
    #[default]
    Warn,
    Strict,
}

#[derive(Debug, Clone)]
pub struct PromptComposerOptions {
    pub enable_diagnostics: bool,
    pub validation_level: ValidationLevel,
    pub cache_ttl: Duration,
}

impl Default for PromptComposerOptions {
    fn default() -> Self {
        Self {
            enable_diagnostics: true,
            validation_level: ValidationLevel::Warn,
            cache_ttl: Duration::from_secs(0),
        }
    }
}

pub struct PromptComposer {
    contributors: Vec<Arc<dyn PromptContributor>>,
    options: PromptComposerOptions,
    contributor_cache: Mutex<HashMap<String, ContributorCacheEntry>>,
}

#[derive(Debug, Clone)]
pub struct PromptBuildOutput {
    pub plan: PromptPlan,
    pub diagnostics: PromptDiagnostics,
}

#[derive(Debug, Clone)]
struct ContributorCacheEntry {
    fingerprint: String,
    cached_at: Instant,
    contribution: PromptContribution,
}

#[derive(Debug, Clone)]
struct CandidateBlock {
    spec: BlockSpec,
    contributor_id: &'static str,
    contributor_vars: HashMap<String, String>,
    insertion_order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockStatus {
    Success,
    SkippedCondition,
    FailedValidation,
    FailedRender,
    MissingDependency,
}

impl PromptComposer {
    pub fn new(options: PromptComposerOptions) -> Self {
        Self {
            contributors: Vec::new(),
            options,
            contributor_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_defaults() -> Self {
        Self::with_options(PromptComposerOptions::default())
    }

    pub fn with_options(options: PromptComposerOptions) -> Self {
        Self::new(options)
            .add(Arc::new(IdentityContributor))
            .add(Arc::new(EnvironmentContributor))
            .add(Arc::new(AgentsMdContributor))
            .add(Arc::new(SkillSummaryContributor))
    }

    pub fn add(mut self, contributor: Arc<dyn PromptContributor>) -> Self {
        self.contributors.push(contributor);
        self
    }

    pub async fn build(&self, ctx: &PromptContext) -> Result<PromptBuildOutput> {
        let mut diagnostics = PromptDiagnostics::default();
        let mut plan = PromptPlan::default();
        let mut candidates = Vec::new();
        let mut insertion_order = 0usize;

        for contributor in &self.contributors {
            let contribution = self
                .collect_contribution(contributor.as_ref(), ctx, &mut diagnostics)
                .await?;

            append_unique_tools(&mut plan.extra_tools, contribution.extra_tools.clone());

            for spec in contribution.blocks {
                candidates.push(CandidateBlock {
                    spec,
                    contributor_id: contributor.contributor_id(),
                    contributor_vars: contribution.contributor_vars.clone(),
                    insertion_order,
                });
                insertion_order += 1;
            }
        }

        let candidates = self.filter_duplicate_block_ids(candidates, &mut diagnostics)?;
        self.resolve_candidates(candidates, ctx, &mut plan, &mut diagnostics)?;

        Ok(PromptBuildOutput { plan, diagnostics })
    }

    async fn collect_contribution(
        &self,
        contributor: &dyn PromptContributor,
        ctx: &PromptContext,
        diagnostics: &mut PromptDiagnostics,
    ) -> Result<PromptContribution> {
        let fingerprint = format!(
            "{}:{}:{}",
            contributor.contributor_id(),
            contributor.cache_version(),
            contributor.cache_fingerprint(ctx)
        );

        if let Some(hit) = self.lookup_cache(contributor.contributor_id(), &fingerprint) {
            self.push_diagnostic(
                diagnostics,
                DiagnosticLevel::Info,
                None,
                Some(contributor.contributor_id().to_string()),
                DiagnosticReason::ContributorCacheHit,
                None,
            );
            return Ok(hit);
        }

        self.push_diagnostic(
            diagnostics,
            DiagnosticLevel::Info,
            None,
            Some(contributor.contributor_id().to_string()),
            DiagnosticReason::ContributorCacheMiss,
            None,
        );

        let contribution = contributor.contribute(ctx).await;
        self.store_cache(
            contributor.contributor_id(),
            fingerprint,
            contribution.clone(),
        );
        Ok(contribution)
    }

    fn lookup_cache(&self, contributor_id: &str, fingerprint: &str) -> Option<PromptContribution> {
        let cache = self
            .contributor_cache
            .lock()
            .expect("contributor cache lock should work");
        let entry = cache.get(contributor_id)?;
        let ttl_valid =
            self.options.cache_ttl.is_zero() || entry.cached_at.elapsed() <= self.options.cache_ttl;

        if entry.fingerprint == fingerprint && ttl_valid {
            Some(entry.contribution.clone())
        } else {
            None
        }
    }

    fn store_cache(
        &self,
        contributor_id: &str,
        fingerprint: String,
        contribution: PromptContribution,
    ) {
        let mut cache = self
            .contributor_cache
            .lock()
            .expect("contributor cache lock should work");
        cache.insert(
            contributor_id.to_string(),
            ContributorCacheEntry {
                fingerprint,
                cached_at: Instant::now(),
                contribution,
            },
        );
    }

    fn filter_duplicate_block_ids(
        &self,
        candidates: Vec<CandidateBlock>,
        diagnostics: &mut PromptDiagnostics,
    ) -> Result<Vec<CandidateBlock>> {
        let mut seen = HashSet::new();
        let mut filtered = Vec::new();

        for candidate in candidates {
            let block_id = candidate.spec.id.to_string();
            if seen.insert(block_id.clone()) {
                filtered.push(candidate);
                continue;
            }

            self.handle_failure(
                diagnostics,
                Some(block_id),
                Some(candidate.contributor_id.to_string()),
                DiagnosticReason::ValidationFailed {
                    message: "duplicate block id".to_string(),
                },
                Some("Ensure each block id is unique within the composed prompt.".to_string()),
                candidate.spec.validation_policy,
            )?;
        }

        Ok(filtered)
    }

    fn resolve_candidates(
        &self,
        candidates: Vec<CandidateBlock>,
        ctx: &PromptContext,
        plan: &mut PromptPlan,
        diagnostics: &mut PromptDiagnostics,
    ) -> Result<()> {
        // 波前式拓扑排序（wave-based topological sort）：
        // 每轮迭代处理所有依赖已就绪的候选块，未就绪的推迟到下一轮。
        // 如果一轮迭代中没有任何进展（progressed == false），说明存在循环依赖。
        // 这种方式比标准 Kahn 算法更简单，且能自然地产生诊断信息。
        let known_ids = candidates
            .iter()
            .map(|candidate| candidate.spec.id.to_string())
            .collect::<HashSet<_>>();
        let mut statuses = HashMap::<String, BlockStatus>::new();
        let mut pending = Vec::new();

        for candidate in candidates {
            if self.condition_matches(&candidate.spec.condition, ctx, &candidate.contributor_vars) {
                pending.push(candidate);
            } else {
                statuses.insert(candidate.spec.id.to_string(), BlockStatus::SkippedCondition);
                self.push_diagnostic(
                    diagnostics,
                    DiagnosticLevel::Info,
                    Some(candidate.spec.id.to_string()),
                    Some(candidate.contributor_id.to_string()),
                    DiagnosticReason::ConditionSkipped {
                        condition: format!("{:?}", candidate.spec.condition),
                    },
                    None,
                );
            }
        }

        while !pending.is_empty() {
            let mut next_pending = Vec::new();
            let mut progressed = false;

            for candidate in pending {
                match self.dependencies_ready(&candidate, &known_ids, &statuses) {
                    DependencyState::Ready => {
                        progressed = true;
                        match self.render_candidate(&candidate, ctx, diagnostics)? {
                            Some(rendered) => {
                                self.push_rendered(plan, rendered, &candidate);
                                statuses
                                    .insert(candidate.spec.id.to_string(), BlockStatus::Success);
                            }
                            None => {
                                let status = if matches!(
                                    candidate.spec.content,
                                    BlockContent::Template(_)
                                ) {
                                    BlockStatus::FailedRender
                                } else {
                                    BlockStatus::FailedValidation
                                };
                                statuses.insert(candidate.spec.id.to_string(), status);
                            }
                        }
                    }
                    DependencyState::Blocked(dependency_id) => {
                        progressed = true;
                        statuses.insert(
                            candidate.spec.id.to_string(),
                            BlockStatus::MissingDependency,
                        );
                        self.push_diagnostic(
                            diagnostics,
                            DiagnosticLevel::Warning,
                            Some(candidate.spec.id.to_string()),
                            Some(candidate.contributor_id.to_string()),
                            DiagnosticReason::MissingDependency { dependency_id },
                            Some(
                                "Check whether the dependency exists and was not skipped or invalidated."
                                    .to_string(),
                            ),
                        );
                    }
                    DependencyState::Pending => next_pending.push(candidate),
                }
            }

            if !progressed {
                for candidate in next_pending {
                    let dependency_id = candidate
                        .spec
                        .dependencies
                        .first()
                        .map(|dependency| dependency.to_string())
                        .unwrap_or_else(|| "<cycle>".to_string());
                    statuses.insert(
                        candidate.spec.id.to_string(),
                        BlockStatus::MissingDependency,
                    );
                    self.push_diagnostic(
                        diagnostics,
                        DiagnosticLevel::Warning,
                        Some(candidate.spec.id.to_string()),
                        Some(candidate.contributor_id.to_string()),
                        DiagnosticReason::MissingDependency { dependency_id },
                        Some(
                            "Dependencies must resolve successfully before the block can render."
                                .to_string(),
                        ),
                    );
                }
                break;
            }

            pending = next_pending;
        }

        Ok(())
    }

    fn condition_matches(
        &self,
        condition: &BlockCondition,
        ctx: &PromptContext,
        contributor_vars: &HashMap<String, String>,
    ) -> bool {
        match condition {
            BlockCondition::Always => true,
            BlockCondition::StepEquals(step) => ctx.step_index == *step,
            BlockCondition::FirstStepOnly => ctx.step_index == 0,
            BlockCondition::HasTool(tool) => ctx.tool_names.iter().any(|name| name == tool),
            BlockCondition::VarEquals { key, expected } => {
                contributor_vars
                    .get(key)
                    .cloned()
                    .or_else(|| ctx.resolve_global_var(key))
                    .or_else(|| ctx.resolve_builtin_var(key))
                    .as_deref()
                    == Some(expected.as_str())
            }
        }
    }

    fn dependencies_ready(
        &self,
        candidate: &CandidateBlock,
        known_ids: &HashSet<String>,
        statuses: &HashMap<String, BlockStatus>,
    ) -> DependencyState {
        for dependency in &candidate.spec.dependencies {
            let dependency_id = dependency.to_string();
            if !known_ids.contains(&dependency_id) {
                return DependencyState::Blocked(dependency_id);
            }

            match statuses.get(&dependency_id) {
                Some(BlockStatus::Success) => {}
                Some(
                    BlockStatus::SkippedCondition
                    | BlockStatus::FailedValidation
                    | BlockStatus::FailedRender
                    | BlockStatus::MissingDependency,
                ) => return DependencyState::Blocked(dependency_id),
                None => return DependencyState::Pending,
            }
        }

        DependencyState::Ready
    }

    fn render_candidate(
        &self,
        candidate: &CandidateBlock,
        ctx: &PromptContext,
        diagnostics: &mut PromptDiagnostics,
    ) -> Result<Option<String>> {
        let rendered = match &candidate.spec.content {
            BlockContent::Text(content) => content.clone(),
            BlockContent::Template(template) => match template.render(|key| {
                candidate
                    .spec
                    .vars
                    .get(key)
                    .cloned()
                    .or_else(|| candidate.contributor_vars.get(key).cloned())
                    .or_else(|| ctx.resolve_global_var(key))
                    .or_else(|| ctx.resolve_builtin_var(key))
            }) {
                Ok(content) => content,
                Err(TemplateRenderError::MissingVariable(variable)) => {
                    self.handle_failure(
                        diagnostics,
                        Some(candidate.spec.id.to_string()),
                        Some(candidate.contributor_id.to_string()),
                        DiagnosticReason::TemplateVariableMissing { variable },
                        Some(
                            "Provide the variable in block vars, contributor vars, PromptContext vars, or builtins."
                                .to_string(),
                        ),
                        candidate.spec.validation_policy,
                    )?;
                    return Ok(None);
                }
                Err(error) => {
                    self.handle_failure(
                        diagnostics,
                        Some(candidate.spec.id.to_string()),
                        Some(candidate.contributor_id.to_string()),
                        DiagnosticReason::RenderFailed {
                            message: error.to_string(),
                        },
                        None,
                        candidate.spec.validation_policy,
                    )?;
                    return Ok(None);
                }
            },
        };

        if !self.validate_render_target(&candidate.spec) {
            self.handle_failure(
                diagnostics,
                Some(candidate.spec.id.to_string()),
                Some(candidate.contributor_id.to_string()),
                DiagnosticReason::ValidationFailed {
                    message: "few-shot blocks must render as prepend/append messages".to_string(),
                },
                None,
                candidate.spec.validation_policy,
            )?;
            return Ok(None);
        }

        if candidate.spec.title.trim().is_empty() {
            self.handle_failure(
                diagnostics,
                Some(candidate.spec.id.to_string()),
                Some(candidate.contributor_id.to_string()),
                DiagnosticReason::ValidationFailed {
                    message: "block title must not be empty".to_string(),
                },
                None,
                candidate.spec.validation_policy,
            )?;
            return Ok(None);
        }

        if rendered.trim().is_empty() {
            self.handle_failure(
                diagnostics,
                Some(candidate.spec.id.to_string()),
                Some(candidate.contributor_id.to_string()),
                DiagnosticReason::ValidationFailed {
                    message: "block content must not be empty".to_string(),
                },
                None,
                candidate.spec.validation_policy,
            )?;
            return Ok(None);
        }

        Ok(Some(rendered))
    }

    fn validate_render_target(&self, spec: &BlockSpec) -> bool {
        !matches!(spec.kind, BlockKind::FewShotExamples)
            || !matches!(spec.render_target, RenderTarget::System)
    }

    fn push_rendered(&self, plan: &mut PromptPlan, rendered: String, candidate: &CandidateBlock) {
        match candidate.spec.render_target {
            RenderTarget::System => plan.system_blocks.push(PromptBlock::new(
                candidate.spec.id.to_string(),
                candidate.spec.kind,
                candidate.spec.title.to_string(),
                rendered,
                candidate.spec.effective_priority(),
                candidate.spec.metadata.clone(),
                candidate.insertion_order,
            )),
            RenderTarget::PrependUser => plan
                .prepend_messages
                .push(LlmMessage::User { content: rendered }),
            RenderTarget::PrependAssistant => plan.prepend_messages.push(LlmMessage::Assistant {
                content: rendered,
                tool_calls: vec![],
                reasoning: None,
            }),
            RenderTarget::AppendUser => plan
                .append_messages
                .push(LlmMessage::User { content: rendered }),
            RenderTarget::AppendAssistant => plan.append_messages.push(LlmMessage::Assistant {
                content: rendered,
                tool_calls: vec![],
                reasoning: None,
            }),
        }
    }

    fn handle_failure(
        &self,
        diagnostics: &mut PromptDiagnostics,
        block_id: Option<String>,
        contributor_id: Option<String>,
        reason: DiagnosticReason,
        suggestion: Option<String>,
        validation_policy: ValidationPolicy,
    ) -> Result<()> {
        match self.effective_validation_level(validation_policy) {
            ValidationLevel::Off => Ok(()),
            ValidationLevel::Warn => {
                self.push_diagnostic(
                    diagnostics,
                    DiagnosticLevel::Warning,
                    block_id,
                    contributor_id,
                    reason,
                    suggestion,
                );
                Ok(())
            }
            ValidationLevel::Strict => Err(anyhow!("prompt block validation failed: {:?}", reason)),
        }
    }

    fn effective_validation_level(&self, validation_policy: ValidationPolicy) -> ValidationLevel {
        match validation_policy {
            ValidationPolicy::Inherit => self.options.validation_level,
            ValidationPolicy::Skip => ValidationLevel::Off,
            ValidationPolicy::Strict => ValidationLevel::Strict,
        }
    }

    fn push_diagnostic(
        &self,
        diagnostics: &mut PromptDiagnostics,
        level: DiagnosticLevel,
        block_id: Option<String>,
        contributor_id: Option<String>,
        reason: DiagnosticReason,
        suggestion: Option<String>,
    ) {
        if !self.options.enable_diagnostics {
            return;
        }

        diagnostics.push(PromptDiagnostic {
            level,
            block_id,
            contributor_id,
            reason,
            suggestion,
            timestamp: chrono::Utc::now(),
        });
    }
}

enum DependencyState {
    Ready,
    Blocked(String),
    Pending,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::time::sleep;

    use super::*;

    fn test_context(working_dir: String) -> PromptContext {
        PromptContext {
            working_dir,
            tool_names: vec!["shell".to_string()],
            step_index: 0,
            turn_index: 0,
            vars: HashMap::new(),
        }
    }

    struct StaticContributor;

    #[async_trait]
    impl PromptContributor for StaticContributor {
        fn contributor_id(&self) -> &'static str {
            "static"
        }

        async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
            PromptContribution {
                blocks: vec![BlockSpec::system_text(
                    "static",
                    BlockKind::Skill,
                    "Skill",
                    "static",
                )],
                ..PromptContribution::default()
            }
        }
    }

    struct CountingContributor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl PromptContributor for CountingContributor {
        fn contributor_id(&self) -> &'static str {
            "counting"
        }

        async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
            self.calls.fetch_add(1, Ordering::SeqCst);
            PromptContribution::default()
        }
    }

    #[tokio::test]
    async fn with_defaults_build_includes_identity_block() {
        let project = tempfile::tempdir().expect("tempdir should be created");
        let composer = PromptComposer::with_defaults();

        let output = composer
            .build(&test_context(project.path().to_string_lossy().into_owned()))
            .await
            .expect("build should succeed");

        assert!(output
            .plan
            .system_blocks
            .iter()
            .any(|block| block.kind == BlockKind::Identity));
    }

    #[tokio::test]
    async fn add_appends_custom_contributor_output() {
        let project = tempfile::tempdir().expect("tempdir should be created");
        let composer = PromptComposer::with_defaults().add(Arc::new(StaticContributor));

        let output = composer
            .build(&test_context(project.path().to_string_lossy().into_owned()))
            .await
            .expect("build should succeed");

        assert!(output
            .plan
            .system_blocks
            .iter()
            .any(|block| block.kind == BlockKind::Skill && block.content == "static"));
    }

    #[tokio::test]
    async fn build_reuses_contributor_cache_for_same_context() {
        let project = tempfile::tempdir().expect("tempdir should be created");
        let calls = Arc::new(AtomicUsize::new(0));
        let composer = PromptComposer::with_defaults().add(Arc::new(CountingContributor {
            calls: calls.clone(),
        }));
        let ctx = test_context(project.path().to_string_lossy().into_owned());

        composer
            .build(&ctx)
            .await
            .expect("first build should succeed");
        composer
            .build(&ctx)
            .await
            .expect("second build should succeed");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn build_expires_contributor_cache_after_ttl() {
        let project = tempfile::tempdir().expect("tempdir should be created");
        let calls = Arc::new(AtomicUsize::new(0));
        let composer = PromptComposer::with_options(PromptComposerOptions {
            cache_ttl: Duration::from_millis(20),
            ..PromptComposerOptions::default()
        })
        .add(Arc::new(CountingContributor {
            calls: calls.clone(),
        }));
        let ctx = test_context(project.path().to_string_lossy().into_owned());

        composer
            .build(&ctx)
            .await
            .expect("first build should succeed");
        sleep(Duration::from_millis(30)).await;
        composer
            .build(&ctx)
            .await
            .expect("second build should succeed");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn template_resolution_prefers_block_then_contributor_then_context_then_builtin() {
        struct TemplateContributor;

        #[async_trait]
        impl PromptContributor for TemplateContributor {
            fn contributor_id(&self) -> &'static str {
                "template"
            }

            async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
                let mut contribution = PromptContribution {
                    blocks: vec![BlockSpec::system_template(
                        "scoped",
                        BlockKind::Skill,
                        "Skill",
                        "{{name}}|{{project.name}}|{{project.working_dir}}|{{env.os}}",
                    )
                    .with_var("name", "block")],
                    ..PromptContribution::default()
                };
                contribution
                    .contributor_vars
                    .insert("project.name".to_string(), "contributor".to_string());
                contribution
            }
        }

        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        })
        .add(Arc::new(TemplateContributor));
        let mut ctx = test_context("/workspace/demo".to_string());
        ctx.vars
            .insert("project.name".to_string(), "context".to_string());

        let output = composer.build(&ctx).await.expect("build should succeed");
        let block = output
            .plan
            .system_blocks
            .iter()
            .find(|block| block.id == "scoped")
            .expect("scoped block should exist");

        assert!(block
            .content
            .starts_with("block|contributor|/workspace/demo|"));
    }

    #[tokio::test]
    async fn missing_dependency_after_condition_skip_emits_diagnostic() {
        struct ConditionalContributor;

        #[async_trait]
        impl PromptContributor for ConditionalContributor {
            fn contributor_id(&self) -> &'static str {
                "conditional"
            }

            async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
                PromptContribution {
                    blocks: vec![
                        BlockSpec::system_text("first", BlockKind::Skill, "First", "first")
                            .with_condition(BlockCondition::StepEquals(99)),
                        BlockSpec::system_text("second", BlockKind::Skill, "Second", "second")
                            .depends_on("first"),
                    ],
                    ..PromptContribution::default()
                }
            }
        }

        let composer = PromptComposer::with_defaults().add(Arc::new(ConditionalContributor));
        let output = composer
            .build(&test_context("/workspace/demo".to_string()))
            .await
            .expect("build should succeed");

        assert!(output
            .diagnostics
            .items
            .iter()
            .any(|item| matches!(item.reason, DiagnosticReason::ConditionSkipped { .. })));
        assert!(output
            .diagnostics
            .items
            .iter()
            .any(|item| matches!(item.reason, DiagnosticReason::MissingDependency { .. })));
    }

    #[tokio::test]
    async fn strict_validation_bubbles_up_error() {
        struct InvalidContributor;

        #[async_trait]
        impl PromptContributor for InvalidContributor {
            fn contributor_id(&self) -> &'static str {
                "invalid"
            }

            async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
                PromptContribution {
                    blocks: vec![BlockSpec::message_text(
                        "few-shot",
                        BlockKind::FewShotExamples,
                        "Few Shot",
                        "bad",
                        RenderTarget::System,
                    )],
                    ..PromptContribution::default()
                }
            }
        }

        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        })
        .add(Arc::new(InvalidContributor));

        let err = composer
            .build(&test_context("/workspace/demo".to_string()))
            .await
            .expect_err("strict validation should fail");
        assert!(err.to_string().contains("prompt block validation failed"));
    }
}
