use async_trait::async_trait;

use crate::prompt::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct EnvironmentContributor;

#[async_trait]
impl PromptContributor for EnvironmentContributor {
    fn contributor_id(&self) -> &'static str {
        "environment"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            blocks: vec![BlockSpec::system_template(
                "environment",
                BlockKind::Environment,
                "Environment",
                "Working directory: {{project.working_dir}}\nOS: {{env.os}}\nDate: {{run.date}}\nAvailable tools: {{tools.names}}",
            )],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OrchestrationConfig, ValidationLevel};
    use crate::prompt::PromptComposer;

    #[tokio::test]
    async fn includes_working_dir_os_date_and_tool_names() {
        let composer = PromptComposer::new(OrchestrationConfig {
            validation_strictness: ValidationLevel::Strict,
            ..OrchestrationConfig::default()
        })
        .add(std::sync::Arc::new(EnvironmentContributor));
        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string(), "readFile".to_string()],
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");
        let block = output
            .plan
            .system_blocks
            .iter()
            .find(|block| block.id == "environment")
            .expect("environment block should exist");
        assert_eq!(block.kind, BlockKind::Environment);
        assert!(block.content.contains("Working directory: /workspace/demo"));
        assert!(block
            .content
            .contains(&format!("OS: {}", std::env::consts::OS)));
        assert!(block.content.contains("Date: "));
        assert!(block.content.contains("Available tools: shell, readFile"));
    }
}
