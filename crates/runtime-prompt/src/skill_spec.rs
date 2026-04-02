use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    #[default]
    Builtin,
    User,
    Project,
    Plugin,
    Mcp,
}

impl SkillSource {
    pub fn as_tag(&self) -> &'static str {
        match self {
            Self::Builtin => "source:builtin",
            Self::User => "source:user",
            Self::Project => "source:project",
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
            return match self.source {
                // Claude-style SKILL.md files do not ship explicit trigger arrays, so we
                // fall back to the skill id/name/description instead of matching every request.
                SkillSource::User | SkillSource::Project => {
                    self.matches_implicit_triggers(latest_user_message)
                }
                _ => true,
            };
        }

        let Some(latest_user_message) = latest_user_message else {
            return false;
        };
        let normalized_message = latest_user_message.to_ascii_lowercase();
        self.triggers
            .iter()
            .any(|trigger| normalized_message.contains(&trigger.to_ascii_lowercase()))
    }

    fn matches_implicit_triggers(&self, latest_user_message: Option<&str>) -> bool {
        let Some(latest_user_message) = latest_user_message else {
            return false;
        };

        let normalized_message = normalize_for_matching(latest_user_message);
        if normalized_message.is_empty() {
            return false;
        }

        let implicit_phrases = self.implicit_trigger_phrases();
        if implicit_phrases
            .iter()
            .any(|phrase| normalized_message.contains(phrase))
        {
            return true;
        }

        let message_tokens = normalized_message.split_whitespace().collect::<Vec<_>>();
        implicit_phrases.iter().any(|phrase| {
            let tokens = phrase
                .split_whitespace()
                .filter(|token| token.len() >= 3)
                .collect::<Vec<_>>();
            tokens.len() >= 2
                && tokens
                    .iter()
                    .all(|token| message_tokens.iter().any(|message| message == token))
        })
    }

    fn implicit_trigger_phrases(&self) -> Vec<String> {
        let mut phrases = Vec::new();
        for candidate in [&self.id, &self.name, &self.description] {
            let normalized = normalize_for_matching(candidate);
            if !normalized.is_empty() && !phrases.iter().any(|existing| existing == &normalized) {
                phrases.push(normalized);
            }
        }

        phrases
    }
}

fn normalize_for_matching(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || (!ch.is_ascii() && ch.is_alphanumeric()) {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_skill(id: &str, name: &str, description: &str) -> SkillSpec {
        SkillSpec {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            guide: "guide".to_string(),
            required_tools: Vec::new(),
            triggers: Vec::new(),
            source: SkillSource::Project,
            expand_tool_guides: false,
        }
    }

    #[test]
    fn project_skill_with_empty_triggers_uses_name_and_description_matching() {
        let skill = project_skill(
            "repo-search",
            "Repo Search",
            "When to use: when the user needs to search the repo",
        );

        assert!(skill.matches_triggers(Some("search the repo for errors")));
        assert!(!skill.matches_triggers(Some("format this file")));
    }
}
