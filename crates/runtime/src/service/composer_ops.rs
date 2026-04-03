//! 输入候选（composer options）查询。
//!
//! 这里显式保留 `skill` 与 `capability` 两种候选来源：
//! - `Skill` capability 只是“按需加载 skill 正文”的工具边界
//! - 具体有哪些 skill，则属于 prompt 资源发现层的数据
//!
//! UI 需要的不是 capability/router 的原始结构，而是一个已经投影好的候选列表，
//! 因此前端候选查询应该在 service 层统一组装，而不是让前端自行拼接多套来源。

use astrcode_core::{CapabilityDescriptor, ToolPromptMetadata};

use super::{
    session_ops::normalize_session_id, ComposerOption, ComposerOptionKind, ComposerOptionsRequest,
    RuntimeService, ServiceResult,
};
use crate::prompt::{SkillSource, SkillSpec};

impl RuntimeService {
    /// 列出某个会话上下文下的输入候选项。
    ///
    /// 会话维度能保证 skill 解析使用正确的 working directory，
    /// 同时也能拿到当前运行时已装配的 capability / prompt surface，
    /// 避免前端重复理解 runtime 内部装配细节。
    pub async fn list_composer_options(
        &self,
        session_id: &str,
        request: ComposerOptionsRequest,
    ) -> ServiceResult<Vec<ComposerOption>> {
        let normalized_session_id = normalize_session_id(session_id);
        let session = self.ensure_session_loaded(&normalized_session_id).await?;
        let current_loop = self.current_loop().await;
        let working_dir = session.working_dir.to_string_lossy().into_owned();

        let mut items = Vec::new();
        let include_all_kinds = request.kinds.is_empty();
        let includes = |kind| include_all_kinds || request.kinds.contains(&kind);

        if includes(ComposerOptionKind::Skill) {
            let mut skills = current_loop
                .skill_catalog()
                .resolve_for_working_dir(&working_dir);
            skills.sort_by(|left, right| left.id.cmp(&right.id));
            items.extend(skills.into_iter().map(skill_to_option));
        }

        if includes(ComposerOptionKind::Capability) {
            let mut descriptors = current_loop.capability_descriptors().to_vec();
            descriptors.sort_by(|left, right| left.name.cmp(&right.name));
            items.extend(descriptors.into_iter().map(capability_to_option));
        }

        let normalized_query = normalize_query(request.query.as_deref());
        if let Some(query) = normalized_query.as_deref() {
            items.retain(|item| option_matches_query(item, query));
        }

        items.sort_by(|left, right| {
            composer_option_kind_rank(left.kind)
                .cmp(&composer_option_kind_rank(right.kind))
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.id.cmp(&right.id))
        });
        if items.len() > request.limit {
            items.truncate(request.limit);
        }

        Ok(items)
    }
}

fn composer_option_kind_rank(kind: ComposerOptionKind) -> u8 {
    match kind {
        ComposerOptionKind::Skill => 0,
        ComposerOptionKind::Capability => 1,
    }
}

fn normalize_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn option_matches_query(option: &ComposerOption, query: &str) -> bool {
    [
        option.id.as_str(),
        option.title.as_str(),
        option.description.as_str(),
    ]
    .into_iter()
    .chain(option.badges.iter().map(String::as_str))
    .chain(option.keywords.iter().map(String::as_str))
    .any(|candidate| candidate.to_ascii_lowercase().contains(query))
}

fn skill_to_option(skill: SkillSpec) -> ComposerOption {
    let source = skill.source.clone();
    let mut badges = vec!["skill".to_string(), skill_source_badge(&source).to_string()];
    if !skill.allowed_tools.is_empty() {
        badges.push(format!("{} tools", skill.allowed_tools.len()));
    }

    let mut keywords = vec![
        skill.name.clone(),
        skill.id.clone(),
        source.as_tag().to_string(),
    ];
    keywords.extend(skill.allowed_tools);
    keywords.extend(skill.asset_files);

    ComposerOption {
        kind: ComposerOptionKind::Skill,
        id: skill.id.clone(),
        title: skill.name,
        description: skill.description,
        // 保留 `/skill-name` 这种显式写法，是为了让前端可以直接把候选
        // 回填进输入框，同时也和 `matches_requested_name` 的 slash-tolerant
        // 规则保持一致。
        insert_text: format!("/{}", skill.id),
        badges,
        keywords,
    }
}

fn skill_source_badge(source: &SkillSource) -> &'static str {
    match source {
        SkillSource::Builtin => "builtin",
        SkillSource::User => "user",
        SkillSource::Project => "project",
        SkillSource::Plugin => "plugin",
        SkillSource::Mcp => "mcp",
    }
}

fn capability_to_option(descriptor: CapabilityDescriptor) -> ComposerOption {
    let descriptor_name = descriptor.name.clone();
    let descriptor_description = descriptor.description.clone();
    let prompt = descriptor
        .metadata
        .get("prompt")
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolPromptMetadata>(value).ok());
    let mut badges = vec![descriptor.kind.to_string()];
    if descriptor.streaming {
        badges.push("streaming".to_string());
    }
    badges.extend(descriptor.profiles.iter().cloned());

    let mut keywords = vec![descriptor.kind.to_string()];
    keywords.extend(descriptor.tags.clone());
    keywords.extend(descriptor.profiles.clone());
    keywords.extend(
        descriptor
            .permissions
            .iter()
            .map(|permission| permission.name.clone()),
    );

    ComposerOption {
        kind: ComposerOptionKind::Capability,
        id: descriptor_name.clone(),
        title: descriptor_name.clone(),
        description: prompt
            .as_ref()
            .map(|prompt| prompt.summary.clone())
            .unwrap_or(descriptor_description),
        insert_text: descriptor
            .metadata
            .get("insertText")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(descriptor_name.as_str())
            .to_string(),
        badges,
        keywords,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use astrcode_core::{Result, ToolContext};
    use astrcode_core::{
        Tool, ToolCapabilityMetadata, ToolDefinition, ToolExecutionResult, ToolPromptMetadata,
        ToolRegistry,
    };

    use crate::prompt::{SkillCatalog, SkillSource, SkillSpec};
    use crate::test_support::{capabilities_from_tools, TestEnvGuard};
    use crate::{ComposerOptionsRequest, RuntimeService};

    use super::ComposerOptionKind;

    struct DemoTool;

    #[async_trait::async_trait]
    impl Tool for DemoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "demo.search".to_string(),
                description: "Search demo data.".to_string(),
                parameters: json!({ "type": "object" }),
            }
        }

        fn capability_metadata(&self) -> ToolCapabilityMetadata {
            ToolCapabilityMetadata::builtin()
                .tag("search")
                .concurrency_safe(true)
                .prompt(ToolPromptMetadata::new(
                    "Search indexed demo data.",
                    "Use it for lookup.",
                ))
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "demo.search".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    fn demo_skill() -> SkillSpec {
        SkillSpec {
            id: "clarify-first".to_string(),
            name: "clarify-first".to_string(),
            description: "Ask clarifying questions before making risky changes.".to_string(),
            guide: "# Clarify".to_string(),
            skill_root: None,
            asset_files: Vec::new(),
            allowed_tools: vec!["readFile".to_string()],
            source: SkillSource::Builtin,
        }
    }

    #[tokio::test]
    async fn list_composer_options_keeps_skill_distinct_from_capability_surface() {
        let _guard = TestEnvGuard::new();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let tools = ToolRegistry::builder().register(Box::new(DemoTool)).build();

        let service = RuntimeService::from_capabilities_with_prompt_inputs(
            capabilities_from_tools(tools),
            Vec::new(),
            Arc::new(SkillCatalog::new(vec![demo_skill()])),
        )
        .expect("service should initialize");
        let session = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");

        let items = service
            .list_composer_options(&session.session_id, ComposerOptionsRequest::default())
            .await
            .expect("composer options should load");

        assert!(items
            .iter()
            .any(|item| { item.kind == ComposerOptionKind::Skill && item.id == "clarify-first" }));
        assert!(items.iter().any(|item| {
            item.kind == ComposerOptionKind::Capability && item.id == "demo.search"
        }));
    }

    #[tokio::test]
    async fn list_composer_options_filters_by_kind_and_query() {
        let _guard = TestEnvGuard::new();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let tools = ToolRegistry::builder().register(Box::new(DemoTool)).build();

        let service = RuntimeService::from_capabilities_with_prompt_inputs(
            capabilities_from_tools(tools),
            Vec::new(),
            Arc::new(SkillCatalog::new(vec![demo_skill()])),
        )
        .expect("service should initialize");
        let session = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");

        let items = service
            .list_composer_options(
                &session.session_id,
                ComposerOptionsRequest {
                    query: Some("clarify".to_string()),
                    kinds: vec![ComposerOptionKind::Skill],
                    limit: 50,
                },
            )
            .await
            .expect("filtered composer options should load");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ComposerOptionKind::Skill);
        assert_eq!(items[0].insert_text, "/clarify-first");
    }
}
