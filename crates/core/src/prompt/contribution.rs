use std::collections::{HashMap, HashSet};

use crate::action::ToolDefinition;

use super::BlockSpec;

#[derive(Default, Clone, Debug)]
pub struct PromptContribution {
    pub blocks: Vec<BlockSpec>,
    pub contributor_vars: HashMap<String, String>,
    pub extra_tools: Vec<ToolDefinition>,
}

impl PromptContribution {
    pub fn merge(&mut self, other: PromptContribution) {
        self.blocks.extend(other.blocks);
        self.contributor_vars.extend(other.contributor_vars);
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
    use crate::prompt::{BlockKind, BlockSpec};

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
            blocks: vec![BlockSpec::system_text(
                "identity",
                BlockKind::Identity,
                "Identity",
                "base",
            )],
            contributor_vars: HashMap::from([("project.name".to_string(), "base".to_string())]),
            extra_tools: vec![tool("shell")],
        };

        base.merge(PromptContribution {
            blocks: vec![BlockSpec::system_text(
                "environment",
                BlockKind::Environment,
                "Environment",
                "env",
            )],
            contributor_vars: HashMap::from([("env.os".to_string(), "windows".to_string())]),
            extra_tools: vec![tool("shell"), tool("grep")],
        });

        assert_eq!(base.blocks.len(), 2);
        assert_eq!(base.contributor_vars.len(), 2);
        assert_eq!(
            base.extra_tools
                .into_iter()
                .map(|definition| definition.name)
                .collect::<Vec<_>>(),
            vec!["shell".to_string(), "grep".to_string()]
        );
    }
}
