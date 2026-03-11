use crate::prompt::{BlockKind, PromptBlock, PromptContext, PromptContribution, PromptContributor};

pub struct EnvironmentContributor;

impl PromptContributor for EnvironmentContributor {
    fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            system_blocks: vec![PromptBlock {
                kind: BlockKind::Environment,
                title: "Environment",
                content: format!(
                    "Working directory: {}\nOS: {}\nDate: {}\nAvailable tools: {}",
                    ctx.working_dir,
                    std::env::consts::OS,
                    chrono::Local::now().format("%Y-%m-%d"),
                    ctx.tool_names.join(", ")
                ),
            }],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_working_dir_os_date_and_tool_names() {
        let contributor = EnvironmentContributor;
        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string(), "readFile".to_string()],
            step_index: 0,
            turn_index: 0,
        };

        let contribution = contributor.contribute(&ctx);

        assert_eq!(contribution.system_blocks.len(), 1);
        let block = &contribution.system_blocks[0];
        assert_eq!(block.kind, BlockKind::Environment);
        assert!(block.content.contains("Working directory: /workspace/demo"));
        assert!(block
            .content
            .contains(&format!("OS: {}", std::env::consts::OS)));
        assert!(block.content.contains("Date: "));
        assert!(block.content.contains("Available tools: shell, readFile"));
    }
}
