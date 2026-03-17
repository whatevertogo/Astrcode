use std::collections::HashSet;

use astrcode_core::{LlmMessage, ToolDefinition};

use super::PromptBlock;

#[derive(Default, Clone, Debug)]
pub struct PromptContribution {
    pub system_blocks: Vec<PromptBlock>,
    pub prepend_messages: Vec<LlmMessage>,
    pub append_messages: Vec<LlmMessage>,
    pub extra_tools: Vec<ToolDefinition>,
}

impl PromptContribution {
    pub fn merge(&mut self, other: PromptContribution) {
        self.system_blocks.extend(other.system_blocks);
        self.prepend_messages.extend(other.prepend_messages);
        self.append_messages.extend(other.append_messages);
        append_unique_tools(&mut self.extra_tools, other.extra_tools);
    }
}

pub fn append_unique_tools(base: &mut Vec<ToolDefinition>, extra: Vec<ToolDefinition>) {
    let mut existing: HashSet<String> = base.iter().map(|tool| tool.name.clone()).collect();

    for tool in extra {
        if existing.insert(tool.name.clone()) {
            base.push(tool);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::prompt::{BlockKind, PromptBlock};
    use astrcode_core::LlmMessage;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("{name} tool"),
            parameters: json!({ "type": "object" }),
        }
    }

    #[test]
    fn append_unique_tools_deduplicates_existing_and_extra_items() {
        let mut base = vec![tool("shell"), tool("readFile")];

        append_unique_tools(
            &mut base,
            vec![
                tool("readFile"),
                tool("grep"),
                tool("grep"),
                tool("shell"),
                tool("findFiles"),
            ],
        );

        let names = base
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "shell".to_string(),
                "readFile".to_string(),
                "grep".to_string(),
                "findFiles".to_string()
            ]
        );
    }

    #[test]
    fn merge_combines_all_fields() {
        let mut base = PromptContribution {
            system_blocks: vec![PromptBlock {
                kind: BlockKind::Identity,
                title: "Identity",
                content: "base".to_string(),
            }],
            prepend_messages: vec![LlmMessage::User {
                content: "before".to_string(),
            }],
            append_messages: vec![LlmMessage::Assistant {
                content: "after".to_string(),
                tool_calls: vec![],
            }],
            extra_tools: vec![tool("shell")],
        };

        base.merge(PromptContribution {
            system_blocks: vec![PromptBlock {
                kind: BlockKind::Environment,
                title: "Environment",
                content: "env".to_string(),
            }],
            prepend_messages: vec![LlmMessage::User {
                content: "extra-before".to_string(),
            }],
            append_messages: vec![LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "tool output".to_string(),
            }],
            extra_tools: vec![tool("shell"), tool("grep")],
        });

        assert_eq!(base.system_blocks.len(), 2);
        assert_eq!(base.prepend_messages.len(), 2);
        assert_eq!(base.append_messages.len(), 2);
        assert_eq!(
            base.extra_tools
                .into_iter()
                .map(|definition| definition.name)
                .collect::<Vec<_>>(),
            vec!["shell".to_string(), "grep".to_string()]
        );
    }
}
