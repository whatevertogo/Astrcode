//! AGENTS.md 贡献者。
//!
//! 从两个位置加载 AGENTS.md 规则文件：
//! - 用户级：`~/.astrcode/AGENTS.md`（适用于所有项目）
//! - 项目级：`<working_dir>/AGENTS.md`（仅适用于当前项目）
//!
//! 两个文件同时存在时都会被包含到 prompt 中，分别作为 UserRules 和 ProjectRules block。

use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::warn;

use super::shared::{cache_marker_for_path, user_astrcode_file_path};
use crate::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

/// AGENTS.md 贡献者。
///
/// 同时加载用户级和项目级 AGENTS.md，分别映射到 `UserRules` 和 `ProjectRules` block。
/// 文件不存在时静默跳过，不阻塞整个 prompt 组装流程。
pub struct AgentsMdContributor;

pub fn user_agents_md_path() -> Option<PathBuf> {
    user_astrcode_file_path("AGENTS.md")
}

pub fn project_agents_md_path(working_dir: &str) -> PathBuf {
    PathBuf::from(working_dir).join("AGENTS.md")
}

pub fn load_agents_md(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(path) {
        Ok(content) => Some(content.trim().to_string()),
        Err(error) => {
            warn!("failed to read {}: {}", path.display(), error);
            None
        }
    }
}

#[async_trait]
impl PromptContributor for AgentsMdContributor {
    fn contributor_id(&self) -> &'static str {
        "agents-md"
    }

    fn cache_version(&self) -> u64 {
        2
    }

    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        let user_marker = user_agents_md_path()
            .map(|path| format!("{}={}", path.display(), cache_marker_for_path(&path)))
            .unwrap_or_else(|| "user=<unresolved>".to_string());
        let project_path = project_agents_md_path(&ctx.working_dir);
        let project_marker = format!(
            "{}={}",
            project_path.display(),
            cache_marker_for_path(&project_path)
        );

        format!("{user_marker}|{project_marker}")
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut blocks = Vec::new();

        if let Some(path) = user_agents_md_path() {
            if let Some(content) = load_agents_md(&path) {
                blocks.push(
                    BlockSpec::system_text(
                        "user-agents-md",
                        BlockKind::UserRules,
                        "User Rules",
                        format!("User-wide instructions from {}:\n{content}", path.display()),
                    )
                    .with_origin(path.display().to_string()),
                );
            }
        }

        let project_path = project_agents_md_path(&ctx.working_dir);
        if let Some(content) = load_agents_md(&project_path) {
            blocks.push(
                BlockSpec::system_text(
                    "project-agents-md",
                    BlockKind::ProjectRules,
                    "Project Rules",
                    format!(
                        "Project-specific instructions from {}:\n{content}",
                        project_path.display()
                    ),
                )
                .with_origin(project_path.display().to_string()),
            );
        }

        PromptContribution {
            blocks,
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use astrcode_core::home::ASTRCODE_HOME_DIR_ENV;
    use astrcode_core::test_support::TestEnvGuard;

    use super::*;
    use crate::BlockContent;

    fn context(working_dir: String) -> PromptContext {
        PromptContext {
            working_dir,
            tool_names: vec!["shell".to_string()],
            capability_descriptors: Vec::new(),
            prompt_declarations: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        }
    }

    #[tokio::test]
    async fn returns_empty_blocks_when_agents_files_are_missing() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let contributor = AgentsMdContributor;

        let contribution = contributor
            .contribute(&context(project.path().to_string_lossy().into_owned()))
            .await;

        assert!(contribution.blocks.is_empty());
    }

    #[tokio::test]
    async fn returns_user_rules_block_with_source_prefix() {
        let guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let user_agents_path = guard.home_dir().join(".astrcode").join("AGENTS.md");
        fs::create_dir_all(user_agents_path.parent().expect("parent should exist"))
            .expect("user agents dir should be created");
        fs::write(&user_agents_path, "Follow user rule")
            .expect("user agents file should be written");
        let contributor = AgentsMdContributor;

        let contribution = contributor
            .contribute(&context(project.path().to_string_lossy().into_owned()))
            .await;

        assert_eq!(contribution.blocks.len(), 1);
        assert_eq!(contribution.blocks[0].kind, BlockKind::UserRules);
        assert!(matches!(
            &contribution.blocks[0].content,
            BlockContent::Text(content)
            if content.contains(&format!(
                "User-wide instructions from {}:\nFollow user rule",
                user_agents_path.display()
            ))
        ));
    }

    #[tokio::test]
    async fn returns_project_rules_block_with_source_prefix() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        fs::write(project.path().join("AGENTS.md"), "Follow project rule")
            .expect("project agents file should be written");
        let contributor = AgentsMdContributor;

        let contribution = contributor
            .contribute(&context(project.path().to_string_lossy().into_owned()))
            .await;

        assert_eq!(contribution.blocks.len(), 1);
        assert_eq!(contribution.blocks[0].kind, BlockKind::ProjectRules);
        assert!(matches!(
            &contribution.blocks[0].content,
            BlockContent::Text(content)
            if content.contains(&format!(
                "Project-specific instructions from {}:\nFollow project rule",
                project.path().join("AGENTS.md").display()
            ))
        ));
    }

    #[tokio::test]
    async fn returns_both_user_and_project_blocks_when_both_exist() {
        let guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let user_agents_path = guard.home_dir().join(".astrcode").join("AGENTS.md");
        fs::create_dir_all(user_agents_path.parent().expect("parent should exist"))
            .expect("user agents dir should be created");
        fs::write(&user_agents_path, "Follow user rule")
            .expect("user agents file should be written");
        fs::write(project.path().join("AGENTS.md"), "Follow project rule")
            .expect("project agents file should be written");
        let contributor = AgentsMdContributor;

        let contribution = contributor
            .contribute(&context(project.path().to_string_lossy().into_owned()))
            .await;

        assert_eq!(contribution.blocks.len(), 2);
        assert!(contribution
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::UserRules));
        assert!(contribution
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::ProjectRules));
    }

    #[test]
    fn user_agents_md_path_prefers_app_home_override() {
        let _guard = TestEnvGuard::new();
        let override_home = tempfile::tempdir().expect("tempdir should be created");
        let previous_override = std::env::var_os(ASTRCODE_HOME_DIR_ENV);

        std::env::set_var(ASTRCODE_HOME_DIR_ENV, override_home.path());
        let path = user_agents_md_path().expect("override path should resolve");

        match previous_override {
            Some(value) => std::env::set_var(ASTRCODE_HOME_DIR_ENV, value),
            None => std::env::remove_var(ASTRCODE_HOME_DIR_ENV),
        }

        assert_eq!(
            path,
            override_home.path().join(".astrcode").join("AGENTS.md")
        );
    }
}
