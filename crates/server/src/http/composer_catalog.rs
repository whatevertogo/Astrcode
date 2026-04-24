//! server-owned composer catalog adapter。
//!
//! 负责把 `plugin-host::ResourceCatalog`、分层 `SkillCatalog` 与当前 runtime capability
//! 快照统一投影成 `host-session::ComposerOption`，从而让 HTTP 路由不再依赖
//! `application::composer` 过渡面。

use astrcode_core::{SessionId, SkillSpec};
use astrcode_host_session::{ComposerOption, ComposerOptionActionKind, ComposerOptionKind};

use crate::{AppState, application_error_bridge::ServerRouteError};

pub(crate) async fn list_session_composer_options(
    state: &AppState,
    session_id: &str,
    query: Option<&str>,
    kinds: &[ComposerOptionKind],
    limit: usize,
) -> Result<Vec<ComposerOption>, ServerRouteError> {
    let working_dir = state
        .session_catalog
        .ensure_loaded_session(&SessionId::from(session_id.to_string()))
        .await
        .map_err(|error| ServerRouteError::internal(error.to_string()))?
        .working_dir
        .display()
        .to_string();
    let mut items = command_options(state);
    items.extend(skill_options(
        state.skill_catalog.resolve_for_working_dir(&working_dir),
    ));
    items.extend(capability_options(state));

    if !kinds.is_empty() {
        items.retain(|item| kinds.contains(&item.kind));
    }

    if let Some(query) = normalize_query(query) {
        items.retain(|item| option_matches_query(item, &query));
    }

    items.truncate(limit);
    Ok(items)
}

fn command_options(state: &AppState) -> Vec<ComposerOption> {
    state
        .resource_catalog
        .read()
        .expect("plugin resource catalog lock poisoned")
        .commands
        .iter()
        .map(|command| ComposerOption {
            kind: ComposerOptionKind::Command,
            id: command.command_id.clone(),
            title: humanize_token_path(&command.command_id),
            description: describe_command(&command.command_id),
            insert_text: format!("/{}", command.command_id),
            action_kind: ComposerOptionActionKind::ExecuteCommand,
            action_value: format!("/{}", command.command_id),
            badges: vec!["command".to_string()],
            keywords: command_keywords(&command.command_id),
        })
        .collect()
}

fn skill_options(skills: Vec<SkillSpec>) -> Vec<ComposerOption> {
    skills
        .into_iter()
        .map(|skill| ComposerOption {
            kind: ComposerOptionKind::Skill,
            id: skill.id.clone(),
            title: humanize_token_path(&skill.id),
            description: skill.description,
            insert_text: format!("/{}", skill.id),
            action_kind: ComposerOptionActionKind::InsertText,
            action_value: format!("/{}", skill.id),
            badges: vec!["skill".to_string(), skill.source.as_tag().to_string()],
            keywords: skill_keywords(&skill.id),
        })
        .collect()
}

fn capability_options(state: &AppState) -> Vec<ComposerOption> {
    state
        .governance
        .capabilities()
        .into_iter()
        .map(|spec| {
            let name = spec.name.to_string();
            ComposerOption {
                kind: ComposerOptionKind::Capability,
                id: name.clone(),
                title: name.clone(),
                description: spec.description,
                insert_text: name.clone(),
                action_kind: ComposerOptionActionKind::InsertText,
                action_value: name.clone(),
                badges: vec!["capability".to_string()],
                keywords: capability_keywords(&name),
            }
        })
        .collect()
}

fn describe_command(command_id: &str) -> String {
    match command_id {
        "compact" => "压缩当前会话上下文".to_string(),
        _ => format!("执行 /{command_id} 命令"),
    }
}

fn command_keywords(command_id: &str) -> Vec<String> {
    let mut keywords = split_keywords(command_id);
    if command_id == "compact" {
        keywords.push("compress".to_string());
    }
    keywords
}

fn skill_keywords(skill_id: &str) -> Vec<String> {
    split_keywords(skill_id)
}

fn capability_keywords(capability_name: &str) -> Vec<String> {
    let mut keywords = split_keywords(capability_name);
    let lowered = capability_name.to_lowercase();
    if !keywords.contains(&lowered) {
        keywords.push(lowered);
    }
    keywords
}

fn split_keywords(value: &str) -> Vec<String> {
    value
        .split(['-', '.', '_', '/', ' '])
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_lowercase())
        .collect()
}

fn humanize_token_path(value: &str) -> String {
    let words = value
        .split(['-', '.', '_', '/'])
        .filter(|segment| !segment.is_empty())
        .map(title_case_token)
        .collect::<Vec<_>>();
    if words.is_empty() {
        value.to_string()
    } else {
        words.join(" ")
    }
}

fn title_case_token(token: &str) -> String {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.as_str();
    if first.is_alphabetic() {
        format!("{}{}", first.to_uppercase(), rest)
    } else {
        format!("{first}{rest}")
    }
}

fn normalize_query(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase())
}

fn option_matches_query(option: &ComposerOption, query: &str) -> bool {
    option.id.to_lowercase().contains(query)
        || option.title.to_lowercase().contains(query)
        || option.description.to_lowercase().contains(query)
        || option
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}
