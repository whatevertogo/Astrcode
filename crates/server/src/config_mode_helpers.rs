use astrcode_core::{
    ActiveSelection, AstrError, Config, CurrentModelSelection, ModelOption, ModelSelection,
    Profile, Result,
};

pub(crate) const PROVIDER_KIND_OPENAI: &str = "openai";
const OPENAI_CHAT_COMPLETIONS_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_RESPONSES_API_URL: &str = "https://api.openai.com/v1/responses";
const LITERAL_VALUE_PREFIX: &str = "literal:";
const ENV_REFERENCE_PREFIX: &str = "env:";

pub(crate) fn resolve_current_model(config: &Config) -> Result<CurrentModelSelection> {
    let selected = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == selected.active_profile)
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "active profile '{}' not found",
                selected.active_profile
            ))
        })?;

    Ok(ModelSelection::new(
        selected.active_profile,
        selected.active_model,
        profile.provider_kind.clone(),
    ))
}

pub(crate) fn list_model_options(config: &Config) -> Vec<ModelOption> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| {
                ModelSelection::new(
                    profile.name.clone(),
                    model.id.clone(),
                    profile.provider_kind.clone(),
                )
            })
        })
        .collect()
}

pub(crate) fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        && value.contains('_')
}

pub(crate) fn resolve_api_key(profile: &Profile) -> Result<String> {
    let value = match &profile.api_key {
        None => {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 未配置 apiKey",
                profile.name
            )));
        },
        Some(value) => value.trim().to_string(),
    };

    if value.is_empty() {
        return Err(AstrError::MissingApiKey(format!(
            "profile '{}' 的 apiKey 不能为空",
            profile.name
        )));
    }

    let resolved = resolve_env_value(&value).map_err(|error| match error {
        AstrError::Validation(message) => {
            AstrError::Validation(format!("profile '{}' 的 apiKey {}", profile.name, message))
        },
        other => other,
    })?;

    if resolved.is_empty() {
        return Err(AstrError::MissingApiKey(format!(
            "profile '{}' 的 apiKey 解析后为空",
            profile.name
        )));
    }

    Ok(resolved)
}

pub(crate) fn resolve_openai_chat_completions_api_url(base_url: &str) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return OPENAI_CHAT_COMPLETIONS_API_URL.to_string();
    }

    let normalized = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if let Some(replaced) = replace_openai_collection_tail(trimmed, "chat/completions") {
        replaced
    } else if let Some(versioned_url) =
        normalize_openai_versioned_base_url(trimmed, "chat/completions")
    {
        versioned_url
    } else {
        format!("{trimmed}/v1/chat/completions")
    };

    join_url_query(normalized, query)
}

pub(crate) fn resolve_openai_responses_api_url(base_url: &str) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return OPENAI_RESPONSES_API_URL.to_string();
    }

    let normalized = if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if let Some(replaced) = replace_openai_collection_tail(trimmed, "responses") {
        replaced
    } else if let Some(versioned_url) = normalize_openai_versioned_base_url(trimmed, "responses") {
        versioned_url
    } else {
        format!("{trimmed}/v1/responses")
    };

    join_url_query(normalized, query)
}

pub(crate) fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[Profile],
) -> Result<ActiveSelection> {
    if profiles.is_empty() {
        return Err(AstrError::Validation("no profiles configured".to_string()));
    }

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(&profiles[0]);

    if selected_profile.name != active_profile {
        return fallback_selection(
            selected_profile,
            format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            ),
        );
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| model.id == active_model)
    {
        return Ok(active_selection(selected_profile, model.id.clone(), None));
    }

    let fallback_model = first_model_id(selected_profile)?.to_string();
    Ok(active_selection(
        selected_profile,
        fallback_model.clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, fallback_model
        )),
    ))
}

fn first_model_id(profile: &Profile) -> Result<&str> {
    profile
        .models
        .first()
        .map(|model| model.id.as_str())
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "profile '{}' has no models configured",
                profile.name
            ))
        })
}

fn fallback_selection(profile: &Profile, warning: String) -> Result<ActiveSelection> {
    Ok(active_selection(
        profile,
        first_model_id(profile)?.to_string(),
        Some(warning),
    ))
}

fn active_selection(
    profile: &Profile,
    active_model: String,
    warning: Option<String>,
) -> ActiveSelection {
    ActiveSelection {
        active_profile: profile.name.clone(),
        active_model,
        warning,
    }
}

fn resolve_env_value(raw: &str) -> Result<String> {
    match parse_env_value(raw)? {
        ParsedEnvValue::Literal(value) => Ok(value.to_string()),
        ParsedEnvValue::ExplicitEnv(env_name) => std::env::var(env_name).map_err(|_| {
            AstrError::EnvVarNotFound(format!(
                "环境变量 {} 未设置。\n解决方案：\n1. \
                 在系统属性中设置用户环境变量（需重启应用）\n2. 或在配置文件中使用 \
                 literal:YOUR_API_KEY 直接指定",
                env_name
            ))
        }),
        ParsedEnvValue::OptionalEnv(env_name) => {
            Ok(std::env::var(env_name).unwrap_or_else(|_| env_name.to_string()))
        },
    }
}

fn parse_env_value(raw: &str) -> Result<ParsedEnvValue<'_>> {
    let trimmed = raw.trim();

    if let Some(literal) = trimmed.strip_prefix(LITERAL_VALUE_PREFIX) {
        return Ok(ParsedEnvValue::Literal(literal.trim()));
    }

    if let Some(env_name) = trimmed.strip_prefix(ENV_REFERENCE_PREFIX) {
        let env_name = env_name.trim();
        if !is_env_var_name(env_name) {
            return Err(AstrError::Validation(format!(
                "env 引用 '{}' 非法",
                env_name
            )));
        }
        return Ok(ParsedEnvValue::ExplicitEnv(env_name));
    }

    if is_env_var_name(trimmed) {
        return Ok(ParsedEnvValue::OptionalEnv(trimmed));
    }

    Ok(ParsedEnvValue::Literal(trimmed))
}

fn looks_like_api_version_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some('v' | 'V'))
        && matches!(chars.next(), Some(ch) if ch.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn normalize_openai_versioned_base_url(trimmed: &str, collection_suffix: &str) -> Option<String> {
    let segments = trimmed.split('/').collect::<Vec<_>>();
    let version_index = segments
        .iter()
        .rposition(|segment| looks_like_api_version_segment(segment))?;
    let prefix = segments[..=version_index].join("/");
    Some(format!("{prefix}/{collection_suffix}"))
}

fn split_url_query(url: &str) -> (&str, Option<&str>) {
    match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    }
}

fn join_url_query(path: String, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    }
}

fn replace_openai_collection_tail(trimmed: &str, collection_suffix: &str) -> Option<String> {
    const KNOWN_SUFFIXES: &[&str] = &[
        "/chat/completions",
        "/chat/completion",
        "/chat",
        "/responses",
        "/response",
    ];

    KNOWN_SUFFIXES.iter().find_map(|suffix| {
        trimmed
            .strip_suffix(suffix)
            .map(|prefix| format!("{prefix}/{collection_suffix}"))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedEnvValue<'a> {
    Literal(&'a str),
    ExplicitEnv(&'a str),
    OptionalEnv(&'a str),
}
