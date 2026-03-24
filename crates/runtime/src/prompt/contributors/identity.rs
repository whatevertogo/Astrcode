use async_trait::async_trait;

use crate::prompt::{BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor};

pub struct IdentityContributor;

const IDENTITY: &str = "\
You are AstrCode, a local AI coding agent running on the user's machine. \
You help with coding tasks, file editing, and terminal commands. \
Be concise and accurate. Prefer editing files directly over explaining how to do it.";

#[async_trait]
impl PromptContributor for IdentityContributor {
    fn contributor_id(&self) -> &'static str {
        "identity"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            blocks: vec![BlockSpec::system_text(
                "identity",
                BlockKind::Identity,
                "Identity",
                IDENTITY,
            )],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(step_index: usize) -> PromptContext {
        PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
            step_index,
            turn_index: 0,
            vars: Default::default(),
        }
    }

    #[tokio::test]
    async fn returns_identity_block_for_any_step_index() {
        let contributor = IdentityContributor;

        for step_index in [0, 1, 5] {
            let contribution = contributor.contribute(&context(step_index)).await;
            assert_eq!(contribution.blocks.len(), 1);
            assert_eq!(contribution.blocks[0].kind, BlockKind::Identity);
            assert_eq!(contribution.blocks[0].title, "Identity");
        }
    }
}
