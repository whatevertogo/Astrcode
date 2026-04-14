//! PromptDeclaration 贡献者。
//!
//! 将外部注入的 `PromptDeclaration` 独立收敛到单独 contributor，
//! 这样 layered builder 可以把继承背景放进专用 `Inherited` 层，
//! 不再和工具指南或项目规则共享缓存段。

use async_trait::async_trait;

use crate::{
    BlockSpec, PromptContext, PromptContribution, PromptContributor, PromptDeclaration,
    prompt_declaration::{
        prompt_declaration_block_kind, prompt_declaration_category,
        prompt_declaration_render_target, prompt_declaration_source_tag,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptDeclarationSegment {
    CompactSummary,
    RecentTail,
    Other,
}

pub struct PromptDeclarationContributor {
    segment: PromptDeclarationSegment,
}

impl PromptDeclarationContributor {
    pub fn compact_summary() -> Self {
        Self {
            segment: PromptDeclarationSegment::CompactSummary,
        }
    }

    pub fn recent_tail() -> Self {
        Self {
            segment: PromptDeclarationSegment::RecentTail,
        }
    }

    pub fn other() -> Self {
        Self {
            segment: PromptDeclarationSegment::Other,
        }
    }

    fn declarations(&self, ctx: &PromptContext) -> Vec<PromptDeclaration> {
        match self.segment {
            PromptDeclarationSegment::CompactSummary => ctx.compact_summary_prompt_declarations(),
            PromptDeclarationSegment::RecentTail => ctx.recent_tail_prompt_declarations(),
            PromptDeclarationSegment::Other => ctx.other_prompt_declarations(),
        }
    }
}

#[async_trait]
impl PromptContributor for PromptDeclarationContributor {
    fn contributor_id(&self) -> &'static str {
        match self.segment {
            PromptDeclarationSegment::CompactSummary => "prompt-declaration-compact-summary",
            PromptDeclarationSegment::RecentTail => "prompt-declaration-recent-tail",
            PromptDeclarationSegment::Other => "prompt-declaration-other",
        }
    }

    fn cache_version(&self) -> u64 {
        2
    }

    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        serde_json::to_string(&self.declarations(ctx))
            .expect("prompt declarations should serialize")
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            blocks: self
                .declarations(ctx)
                .iter()
                .map(build_prompt_declaration_block)
                .collect(),
            ..PromptContribution::default()
        }
    }
}

fn build_prompt_declaration_block(declaration: &PromptDeclaration) -> BlockSpec {
    let mut block = BlockSpec::message_text(
        declaration.block_id.clone(),
        prompt_declaration_block_kind(&declaration.kind),
        declaration.title.clone(),
        declaration.content.clone(),
        prompt_declaration_render_target(&declaration.render_target),
    )
    .with_layer(declaration.layer)
    .with_category(prompt_declaration_category(&declaration.kind))
    .with_tag(prompt_declaration_source_tag(&declaration.source));

    if let Some(priority_hint) = declaration.priority_hint {
        block = block.with_priority(priority_hint);
    }
    if let Some(capability_name) = &declaration.capability_name {
        block = block.with_tag(format!("capability:{capability_name}"));
    }
    if let Some(origin) = &declaration.origin {
        block = block.with_origin(origin.clone());
    }
    block
}

#[cfg(test)]
mod tests {
    use super::PromptDeclarationContributor;
    use crate::{
        BlockKind, PromptContext, PromptContributor, PromptDeclaration, PromptDeclarationKind,
        PromptDeclarationRenderTarget, PromptDeclarationSource, PromptLayer,
    };

    #[tokio::test]
    async fn keeps_prompt_declaration_layer_metadata() {
        let contribution = PromptDeclarationContributor::compact_summary()
            .contribute(&PromptContext {
                prompt_declarations: vec![PromptDeclaration {
                    block_id: "child.inherited.compact_summary".to_string(),
                    title: "Inherited Compact Summary".to_string(),
                    content: "compact summary".to_string(),
                    render_target: PromptDeclarationRenderTarget::System,
                    layer: PromptLayer::Inherited,
                    kind: PromptDeclarationKind::ExtensionInstruction,
                    priority_hint: Some(581),
                    always_include: true,
                    source: PromptDeclarationSource::Builtin,
                    capability_name: None,
                    origin: Some("child-context:compact-summary".to_string()),
                }],
                ..PromptContext::default()
            })
            .await;

        assert_eq!(contribution.blocks.len(), 1);
        let block = &contribution.blocks[0];
        assert_eq!(block.kind, BlockKind::ExtensionInstruction);
        assert_eq!(block.layer, PromptLayer::Inherited);
    }

    #[tokio::test]
    async fn splits_compact_summary_and_recent_tail_into_independent_segments() {
        let ctx = PromptContext {
            prompt_declarations: vec![
                PromptDeclaration {
                    block_id: "child.inherited.compact_summary".to_string(),
                    title: "Inherited Compact Summary".to_string(),
                    content: "summary".to_string(),
                    render_target: PromptDeclarationRenderTarget::System,
                    layer: PromptLayer::Inherited,
                    kind: PromptDeclarationKind::ExtensionInstruction,
                    priority_hint: None,
                    always_include: true,
                    source: PromptDeclarationSource::Builtin,
                    capability_name: None,
                    origin: Some("child-context:compact-summary".to_string()),
                },
                PromptDeclaration {
                    block_id: "child.inherited.recent_tail".to_string(),
                    title: "Inherited Recent Tail".to_string(),
                    content: "tail".to_string(),
                    render_target: PromptDeclarationRenderTarget::System,
                    layer: PromptLayer::Inherited,
                    kind: PromptDeclarationKind::ExtensionInstruction,
                    priority_hint: None,
                    always_include: true,
                    source: PromptDeclarationSource::Builtin,
                    capability_name: None,
                    origin: Some("child-context:recent-tail".to_string()),
                },
            ],
            ..PromptContext::default()
        };

        let compact_summary = PromptDeclarationContributor::compact_summary()
            .contribute(&ctx)
            .await;
        let recent_tail = PromptDeclarationContributor::recent_tail()
            .contribute(&ctx)
            .await;
        let other = PromptDeclarationContributor::other().contribute(&ctx).await;

        assert_eq!(compact_summary.blocks.len(), 1);
        assert_eq!(recent_tail.blocks.len(), 1);
        assert!(other.blocks.is_empty());
        assert_eq!(
            compact_summary.blocks[0].id,
            "child.inherited.compact_summary"
        );
        assert_eq!(recent_tail.blocks[0].id, "child.inherited.recent_tail");
    }
}
