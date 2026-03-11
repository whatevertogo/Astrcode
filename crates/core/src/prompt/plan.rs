use super::{append_unique_tools, PromptBlock, PromptContribution};

#[derive(Default, Clone, Debug)]
pub struct PromptPlan {
    pub system_blocks: Vec<PromptBlock>,
    pub prepend_messages: Vec<crate::action::LlmMessage>,
    pub append_messages: Vec<crate::action::LlmMessage>,
    pub extra_tools: Vec<crate::action::ToolDefinition>,
}

impl PromptPlan {
    pub fn merge(&mut self, contribution: PromptContribution) {
        self.system_blocks.extend(contribution.system_blocks);
        self.prepend_messages.extend(contribution.prepend_messages);
        self.append_messages.extend(contribution.append_messages);
        append_unique_tools(&mut self.extra_tools, contribution.extra_tools);
    }

    pub fn render_system(&self) -> Option<String> {
        if self.system_blocks.is_empty() {
            return None;
        }

        let mut blocks = self.system_blocks.clone();
        blocks.sort_by_key(|block| block.kind.clone());

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
    use crate::prompt::{BlockKind, PromptBlock};

    #[test]
    fn render_system_returns_none_when_empty() {
        assert!(PromptPlan::default().render_system().is_none());
    }

    #[test]
    fn render_system_formats_single_block() {
        let plan = PromptPlan {
            system_blocks: vec![PromptBlock {
                kind: BlockKind::Identity,
                title: "Identity",
                content: "hello".to_string(),
            }],
            ..PromptPlan::default()
        };

        assert_eq!(plan.render_system().as_deref(), Some("[Identity]\nhello"));
    }

    #[test]
    fn render_system_sorts_blocks_and_uses_double_newlines_without_extra_whitespace() {
        let plan = PromptPlan {
            system_blocks: vec![
                PromptBlock {
                    kind: BlockKind::ProjectRules,
                    title: "Project Rules",
                    content: "project".to_string(),
                },
                PromptBlock {
                    kind: BlockKind::Identity,
                    title: "Identity",
                    content: "identity".to_string(),
                },
                PromptBlock {
                    kind: BlockKind::Environment,
                    title: "Environment",
                    content: "environment".to_string(),
                },
            ],
            ..PromptPlan::default()
        };

        let rendered = plan.render_system().expect("system prompt should render");

        assert_eq!(
            rendered,
            "[Identity]\nidentity\n\n[Environment]\nenvironment\n\n[Project Rules]\nproject"
        );
        assert_eq!(rendered.trim(), rendered);
        assert!(!rendered.starts_with('\n'));
        assert!(!rendered.ends_with('\n'));
    }
}
