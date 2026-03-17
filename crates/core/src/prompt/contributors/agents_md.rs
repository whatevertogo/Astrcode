use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::warn;

use crate::prompt::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct AgentsMdContributor;

pub fn user_agents_md_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("ASTRCODE_HOME_DIR") {
        if !home.is_empty() {
            return Some(PathBuf::from(home).join(".astrcode").join("AGENTS.md"));
        }
    }

    #[cfg(test)]
    if let Some(home) = crate::test_support::test_home_dir() {
        return Some(home.join(".astrcode").join("AGENTS.md"));
    }

    match dirs::home_dir() {
        Some(home) => Some(home.join(".astrcode").join("AGENTS.md")),
        None => {
            warn!("failed to resolve home dir for AGENTS.md");
            None
        }
    }
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

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut blocks = Vec::new();

        if let Some(path) = user_agents_md_path() {
            if let Some(content) = load_agents_md(&path) {
                blocks.push(
                    BlockSpec::system_text(
                        "user-rules",
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
                    "project-rules",
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

    use super::*;
    use crate::test_support::TestEnvGuard;

    fn context(working_dir: String) -> PromptContext {
        PromptContext {
            working_dir,
            tool_names: vec!["shell".to_string()],
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
            crate::prompt::BlockContent::Text(content)
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
    }

    #[test]
    fn user_agents_md_path_prefers_app_home_override() {
        let _guard = TestEnvGuard::new();
        let override_home = tempfile::tempdir().expect("tempdir should be created");
        let previous_override = std::env::var_os("ASTRCODE_HOME_DIR");

        std::env::set_var("ASTRCODE_HOME_DIR", override_home.path());
        let path = user_agents_md_path().expect("override path should resolve");

        match previous_override {
            Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
            None => std::env::remove_var("ASTRCODE_HOME_DIR"),
        }

        assert_eq!(
            path,
            override_home.path().join(".astrcode").join("AGENTS.md")
        );
    }
}
