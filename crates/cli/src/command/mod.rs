use astrcode_client::{
    AstrcodeConversationSlashActionKindDto, AstrcodeConversationSlashCandidateDto,
};

use crate::state::PaletteSelection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    SubmitPrompt { text: String },
    RunCommand(Command),
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    New,
    Resume { query: Option<String> },
    Compact,
    Skill { query: Option<String> },
    Unknown { raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    SwitchSession { session_id: String },
    ReplaceInput { text: String },
    RunCommand(Command),
}

pub fn classify_input(input: &str) -> InputAction {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return InputAction::Empty;
    }

    if !trimmed.starts_with('/') {
        return InputAction::SubmitPrompt {
            text: trimmed.to_string(),
        };
    }

    InputAction::RunCommand(parse_command(trimmed))
}

pub fn palette_action(selection: PaletteSelection) -> PaletteAction {
    match selection {
        PaletteSelection::ResumeSession(session_id) => PaletteAction::SwitchSession { session_id },
        PaletteSelection::SlashCandidate(candidate) => match candidate.action_kind {
            AstrcodeConversationSlashActionKindDto::InsertText => PaletteAction::ReplaceInput {
                text: candidate.action_value,
            },
            AstrcodeConversationSlashActionKindDto::ExecuteCommand => {
                PaletteAction::RunCommand(parse_command(candidate.action_value.as_str()))
            },
        },
    }
}

pub fn parse_command(command: &str) -> Command {
    let trimmed = command.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or_default();
    let tail = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    match head {
        "/new" => Command::New,
        "/resume" => Command::Resume { query: tail },
        "/compact" => Command::Compact,
        "/skill" => Command::Skill { query: tail },
        _ => Command::Unknown {
            raw: trimmed.to_string(),
        },
    }
}

pub fn filter_slash_candidates(
    candidates: &[AstrcodeConversationSlashCandidateDto],
    query: &str,
) -> Vec<AstrcodeConversationSlashCandidateDto> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return candidates.to_vec();
    }

    candidates
        .iter()
        .filter(|candidate| {
            candidate.id.to_lowercase().contains(&query)
                || candidate.title.to_lowercase().contains(&query)
                || candidate.description.to_lowercase().contains(&query)
                || candidate
                    .keywords
                    .iter()
                    .any(|keyword| keyword.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_built_in_commands() {
        assert_eq!(parse_command("/new"), Command::New);
        assert_eq!(
            parse_command("/resume terminal"),
            Command::Resume {
                query: Some("terminal".to_string())
            }
        );
        assert_eq!(
            parse_command("/skill review"),
            Command::Skill {
                query: Some("review".to_string())
            }
        );
    }

    #[test]
    fn classifies_plain_prompt_without_command_semantics() {
        assert_eq!(
            classify_input("实现 terminal v1"),
            InputAction::SubmitPrompt {
                text: "实现 terminal v1".to_string()
            }
        );
    }
}
