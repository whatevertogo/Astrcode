use astrcode_core::{LlmMessage, ToolDefinition};

use super::PromptBlock;

#[derive(Default, Clone, Debug)]
pub struct PromptPlan {
    pub system_blocks: Vec<PromptBlock>,
    pub prepend_messages: Vec<LlmMessage>,
    pub append_messages: Vec<LlmMessage>,
    pub extra_tools: Vec<ToolDefinition>,
}

impl PromptPlan {
    pub fn render_system(&self) -> Option<String> {
        if self.system_blocks.is_empty() {
            return None;
        }

        let mut blocks: Vec<&PromptBlock> = self.system_blocks.iter().collect();
        blocks.sort_by_key(|block| (block.priority, block.insertion_order));

        Some(
            blocks
                .into_iter()
                .map(|block| format!("[{}]\n{}", block.title, block.content))
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockMetadata;
    use crate::{BlockKind, PromptBlock};

    #[test]
    fn render_system_returns_none_when_empty() {
        assert!(PromptPlan::default().render_system().is_none());
    }

    #[test]
    fn render_system_formats_single_block() {
        let plan = PromptPlan {
            system_blocks: vec![PromptBlock::new(
                "identity",
                BlockKind::Identity,
                "Identity",
                "hello",
                100,
                BlockMetadata::default(),
                0,
            )],
            ..PromptPlan::default()
        };

        assert_eq!(plan.render_system().as_deref(), Some("[Identity]\nhello"));
    }

    #[test]
    fn render_system_sorts_blocks_by_priority_and_insertion_order() {
        let plan = PromptPlan {
            system_blocks: vec![
                PromptBlock::new(
                    "project",
                    BlockKind::ProjectRules,
                    "Project Rules",
                    "project",
                    500,
                    BlockMetadata::default(),
                    2,
                ),
                PromptBlock::new(
                    "identity",
                    BlockKind::Identity,
                    "Identity",
                    "identity",
                    100,
                    BlockMetadata::default(),
                    1,
                ),
                PromptBlock::new(
                    "environment",
                    BlockKind::Environment,
                    "Environment",
                    "environment",
                    100,
                    BlockMetadata::default(),
                    0,
                ),
            ],
            ..PromptPlan::default()
        };

        let rendered = plan.render_system().expect("system prompt should render");

        assert_eq!(
            rendered,
            "[Environment]\nenvironment\n\n[Identity]\nidentity\n\n[Project Rules]\nproject"
        );
    }
}
