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
    pub skill_root: Option<String>,
    #[serde(default)]
    pub reference_files: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default)]
    pub expand_tool_guides: bool,
}

impl SkillSpec {
    pub fn matches(&self, tool_names: &[String], latest_user_message: Option<&str>) -> bool {
        self.matches_allowed_tools(tool_names) && self.matches_triggers(latest_user_message)
    }

    pub fn matches_allowed_tools(&self, tool_names: &[String]) -> bool {
        self.allowed_tools
            .iter()
            .all(|required| tool_names.iter().any(|tool_name| tool_name == required))
    }

    pub fn matches_triggers(&self, latest_user_message: Option<&str>) -> bool {
        if self.triggers.is_empty() {
            // Claude-style SKILL.md files do not rely on explicit trigger arrays.
            // When no manual triggers are present, every skill source falls back to
            // id/name/description matching so bundled and custom skills share one
            // activation model instead of two subtly different systems.
            return self.matches_implicit_triggers(latest_user_message);
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
            for phrase in split_candidate_phrases(candidate) {
                if !phrases.iter().any(|existing| existing == &phrase) {
                    phrases.push(phrase);
                }
            }
        }

        phrases
    }
}

fn split_candidate_phrases(value: &str) -> Vec<String> {
    value
        .split(['\n', ',', ';', '.', '(', ')'])
        .map(normalize_for_matching)
        .filter(|phrase| !phrase.is_empty())
        .collect()
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
            skill_root: None,
            reference_files: Vec::new(),
            allowed_tools: Vec::new(),
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

    #[test]
    fn builtin_skill_with_empty_triggers_uses_implicit_matching_too() {
        let skill = SkillSpec {
            id: "code-modification".to_string(),
            name: "Code Modification".to_string(),
            description:
                "When to use: when the user asks to fix, implement, refactor, or otherwise modify code"
                    .to_string(),
            guide: "guide".to_string(),
            skill_root: None,
            reference_files: Vec::new(),
            allowed_tools: Vec::new(),
            triggers: Vec::new(),
            source: SkillSource::Builtin,
            expand_tool_guides: false,
        };

        assert!(skill.matches_triggers(Some("please refactor this module")));
        assert!(!skill.matches_triggers(Some("list my sessions")));
    }
}
