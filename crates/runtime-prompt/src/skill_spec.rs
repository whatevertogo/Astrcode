use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    #[default]
    Builtin,
    Plugin,
    Mcp,
}

impl SkillSource {
    pub fn as_tag(&self) -> &'static str {
        match self {
            Self::Builtin => "source:builtin",
            Self::Plugin => "source:plugin",
            Self::Mcp => "source:mcp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSpec {
    pub id: String,
    pub name: String,
    pub description: String,
    pub guide: String,
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default)]
    pub expand_tool_guides: bool,
}

impl SkillSpec {
    pub fn matches(&self, tool_names: &[String], latest_user_message: Option<&str>) -> bool {
        self.matches_required_tools(tool_names) && self.matches_triggers(latest_user_message)
    }

    pub fn matches_required_tools(&self, tool_names: &[String]) -> bool {
        self.required_tools
            .iter()
            .all(|required| tool_names.iter().any(|tool_name| tool_name == required))
    }

    pub fn matches_triggers(&self, latest_user_message: Option<&str>) -> bool {
        if self.triggers.is_empty() {
            return true;
        }

        let Some(latest_user_message) = latest_user_message else {
            return false;
        };
        let normalized_message = latest_user_message.to_ascii_lowercase();
        self.triggers
            .iter()
            .any(|trigger| normalized_message.contains(&trigger.to_ascii_lowercase()))
    }
}
