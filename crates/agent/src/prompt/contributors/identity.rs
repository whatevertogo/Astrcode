use crate::prompt::{BlockKind, PromptBlock, PromptContext, PromptContribution, PromptContributor};

pub struct IdentityContributor;

const IDENTITY: &str = "\
You are AstrCode, a local AI coding agent running on the user's machine. \
You help with coding tasks, file editing, and terminal commands. \
Be concise and accurate. Prefer editing files directly over explaining how to do it.";

impl PromptContributor for IdentityContributor {
    fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            system_blocks: vec![PromptBlock {
                kind: BlockKind::Identity,
                title: "Identity",
                content: IDENTITY.to_string(),
            }],
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
        }
    }

    #[test]
    fn returns_identity_block_for_any_step_index() {
        let contributor = IdentityContributor;

        for step_index in [0, 1, 5] {
            let contribution = contributor.contribute(&context(step_index));
            assert_eq!(contribution.system_blocks.len(), 1);
            assert_eq!(contribution.system_blocks[0].kind, BlockKind::Identity);
            assert_eq!(contribution.system_blocks[0].title, "Identity");
            assert_eq!(contribution.system_blocks[0].content, IDENTITY);
        }
    }
}
