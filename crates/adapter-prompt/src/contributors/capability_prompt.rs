//! 能力（工具）指南贡献者。
//!
//! 从 [`CapabilitySpec`] 中提取工具的 prompt 元数据，
//! 生成工具摘要块和详细指南块。
//!
//! # 设计原则
//!
//! - 当工具数量 ≤ 4 时，展开所有工具的详细指南
//! - 超过 4 个工具时，仅展开标记为 `always_include` 的工具
//! - 只负责工具指南；外部 `PromptDeclaration` 由独立 contributor 承接

use astrcode_core::{CapabilitySpec, ToolPromptMetadata};
use async_trait::async_trait;

use crate::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct CapabilityPromptContributor;

const MAX_ALWAYS_ON_DETAILED_GUIDES: usize = 4;

#[async_trait]
impl PromptContributor for CapabilityPromptContributor {
    fn contributor_id(&self) -> &'static str {
        "capability-prompt"
    }

    fn cache_version(&self) -> u64 {
        4
    }

    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        ctx.contributor_cache_fingerprint()
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut blocks = Vec::new();

        // 带 prompt 元数据的工具（通常为 builtin）
        let tool_guides = collect_tool_guides(&ctx.capability_specs);
        // 标记为 source:mcp 或 source:plugin 的外部工具（无 prompt 元数据）
        let external_tools = collect_external_tools(&ctx.capability_specs);

        if !tool_guides.is_empty() || !external_tools.is_empty() {
            blocks.push(build_tool_summary_block(&tool_guides, &external_tools));
        }

        // 外部工具不展开详细指南（仅出现在 summary）
        blocks.extend(
            tool_guides
                .iter()
                .filter(|guide| {
                    guide.prompt.always_include || should_expand_tool_guides(tool_guides.len())
                })
                .map(build_detailed_tool_block),
        );

        PromptContribution {
            blocks,
            ..PromptContribution::default()
        }
    }
}

#[derive(Clone)]
struct ToolGuideEntry {
    spec: CapabilitySpec,
    prompt: ToolPromptMetadata,
}

fn collect_tool_guides(capability_specs: &[CapabilitySpec]) -> Vec<ToolGuideEntry> {
    let mut guides = capability_specs
        .iter()
        .filter(|spec| spec.kind.is_tool())
        .filter_map(|spec| {
            let prompt =
                spec.metadata.get("prompt").cloned().and_then(
                    |value| match serde_json::from_value::<ToolPromptMetadata>(value) {
                        Ok(prompt) => Some(prompt),
                        Err(error) => {
                            log::warn!(
                                "ignoring invalid prompt metadata for capability '{}': {}",
                                spec.name,
                                error
                            );
                            None
                        },
                    },
                )?;
            Some(ToolGuideEntry {
                spec: spec.clone(),
                prompt,
            })
        })
        .collect::<Vec<_>>();
    guides.sort_by(|left, right| left.spec.name.as_str().cmp(right.spec.name.as_str()));
    guides
}

/// 收集标记为 source:mcp 或 source:plugin 的外部工具（无 prompt 元数据）。
///
/// 这些工具仅出现在摘要索引中，不展开详细指南。
fn collect_external_tools(capability_specs: &[CapabilitySpec]) -> Vec<CapabilitySpec> {
    let mut tools: Vec<CapabilitySpec> = capability_specs
        .iter()
        .filter(|spec| spec.kind.is_tool())
        .filter(|spec| {
            spec.tags
                .iter()
                .any(|t| t == "source:mcp" || t == "source:plugin")
        })
        .filter(|spec| {
            // 排除已有 prompt 元数据的（已经由 tool_guides 处理）
            spec.metadata.get("prompt").is_none()
        })
        .cloned()
        .collect();
    tools.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
    tools
}

fn should_expand_tool_guides(tool_guide_count: usize) -> bool {
    tool_guide_count <= MAX_ALWAYS_ON_DETAILED_GUIDES
}

fn build_tool_summary_block(
    tool_guides: &[ToolGuideEntry],
    external_tools: &[CapabilitySpec],
) -> BlockSpec {
    let mut content = String::from(
        "Use the narrowest tool that can answer the request. Prefer read-only inspection before \
         mutation. All paths must stay inside the working directory.\n",
    );
    for guide in tool_guides {
        let caveat = guide
            .prompt
            .caveats
            .first()
            .map(|caveat| format!(" Caveat: {caveat}"))
            .unwrap_or_default();
        content.push_str(&format!(
            "\n- `{}`: {}{}",
            guide.spec.name, guide.prompt.summary, caveat
        ));
    }

    // 外部工具（MCP/Plugin）仅一行 name: description
    for tool in external_tools {
        content.push_str(&format!("\n- `{}`: {}", tool.name, tool.description));
    }

    BlockSpec::system_text(
        "tool-summary",
        BlockKind::ToolGuide,
        "Tool Summary",
        content,
    )
    .with_tag("source:capability")
    .with_category("capabilities")
}

fn build_detailed_tool_block(guide: &ToolGuideEntry) -> BlockSpec {
    let mut sections = vec![guide.prompt.guide.clone()];
    if !guide.prompt.caveats.is_empty() {
        sections.push(format!(
            "Caveats:\n{}",
            guide
                .prompt
                .caveats
                .iter()
                .map(|caveat| format!("- {caveat}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !guide.prompt.examples.is_empty() {
        sections.push(format!(
            "Examples:\n{}",
            guide
                .prompt
                .examples
                .iter()
                .map(|example| format!("- {example}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let mut block = BlockSpec::system_text(
        format!("tool-guide-{}", guide.spec.name),
        BlockKind::ToolGuide,
        format!("Tool Guide: {}", guide.spec.name),
        sections.join("\n\n"),
    )
    .with_category("capabilities")
    .with_tag("source:capability")
    .with_tag(format!("capability:{}", guide.spec.name));
    if let Some(origin) = guide
        .spec
        .metadata
        .get("origin")
        .and_then(serde_json::Value::as_str)
    {
        block = block.with_origin(origin.to_string());
    }
    block
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        CapabilityKind, CapabilitySpec, ToolPromptMetadata, test_support::TestEnvGuard,
    };
    use serde_json::json;

    use super::*;

    fn tool_spec(name: &str, always_include: bool) -> CapabilitySpec {
        CapabilitySpec::builder(name, CapabilityKind::Tool)
            .description(format!("spec for {name}"))
            .schema(json!({"type": "object"}), json!({"type": "string"}))
            .metadata(json!({
                "prompt": ToolPromptMetadata::new(
                    format!("{name} summary"),
                    format!("{name} detailed guide")
                )
                .caveat(format!("{name} caveat"))
                .example(format!("{name} example"))
                .always_include(always_include)
            }))
            .build()
            .expect("spec should build")
    }

    fn context() -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string(), "grep".to_string()],
            capability_specs: vec![tool_spec("shell", false), tool_spec("grep", false)],
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        }
    }

    #[tokio::test]
    async fn contributes_tool_summary_and_detailed_guides() {
        let contribution = CapabilityPromptContributor.contribute(&context()).await;

        assert!(
            contribution
                .blocks
                .iter()
                .any(|block| block.id == "tool-summary" && block.kind == BlockKind::ToolGuide)
        );
        assert!(
            contribution
                .blocks
                .iter()
                .any(|block| block.id == "tool-guide-grep" && block.kind == BlockKind::ToolGuide)
        );
    }

    #[tokio::test]
    async fn large_tool_surfaces_only_expand_always_include_guides() {
        let _guard = TestEnvGuard::new();
        let mut ctx = context();
        ctx.capability_specs = vec![
            tool_spec("alpha", false),
            tool_spec("beta", false),
            tool_spec("gamma", false),
            tool_spec("delta", false),
            tool_spec("epsilon", true),
        ];

        let contribution = CapabilityPromptContributor.contribute(&ctx).await;

        assert!(
            contribution
                .blocks
                .iter()
                .any(|block| block.id == "tool-guide-epsilon")
        );
        assert!(
            !contribution
                .blocks
                .iter()
                .any(|block| block.id == "tool-guide-alpha")
        );
    }
}
