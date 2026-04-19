use super::*;

pub(super) fn sanitize_compact_summary(summary: &str) -> String {
    let had_route_sensitive_content = summary_has_route_sensitive_content(summary);
    let mut sanitized = summary.trim().to_string();
    sanitized = direct_child_validation_regex()
        .replace_all(
            &sanitized,
            "direct-child validation rejected a stale child reference; use the live direct-child \
             snapshot or the latest live tool result instead.",
        )
        .into_owned();
    sanitized = child_agent_reference_block_regex()
        .replace_all(
            &sanitized,
            "Child agent reference metadata existed earlier, but compacted history is not an \
             authoritative routing source.",
        )
        .into_owned();
    for (regex, replacement) in [
        (
            route_key_regex("agentId"),
            "${key}<latest-direct-child-agentId>",
        ),
        (
            route_key_regex("childAgentId"),
            "${key}<latest-direct-child-agentId>",
        ),
        (route_key_regex("parentAgentId"), "${key}<parent-agentId>"),
        (route_key_regex("subRunId"), "${key}<direct-child-subRunId>"),
        (route_key_regex("parentSubRunId"), "${key}<parent-subRunId>"),
        (route_key_regex("sessionId"), "${key}<session-id>"),
        (
            route_key_regex("childSessionId"),
            "${key}<child-session-id>",
        ),
        (route_key_regex("openSessionId"), "${key}<child-session-id>"),
    ] {
        sanitized = regex.replace_all(&sanitized, replacement).into_owned();
    }
    sanitized = exact_agent_instruction_regex()
        .replace_all(
            &sanitized,
            "Use only the latest live child snapshot or tool result for agent routing.",
        )
        .into_owned();
    sanitized = raw_root_agent_id_regex()
        .replace_all(&sanitized, "<agent-id>")
        .into_owned();
    sanitized = raw_agent_id_regex()
        .replace_all(&sanitized, "<agent-id>")
        .into_owned();
    sanitized = raw_subrun_id_regex()
        .replace_all(&sanitized, "<subrun-id>")
        .into_owned();
    sanitized = raw_session_id_regex()
        .replace_all(&sanitized, "<session-id>")
        .into_owned();
    sanitized = collapse_compaction_whitespace(&sanitized);
    if had_route_sensitive_content {
        ensure_compact_boundary_section(&sanitized)
    } else {
        sanitized
    }
}

pub(super) fn sanitize_recent_user_context_digest(digest: &str) -> String {
    collapse_compaction_whitespace(digest)
}

fn ensure_compact_boundary_section(summary: &str) -> String {
    if summary.contains("## Compact Boundary") {
        return summary.to_string();
    }
    format!(
        "## Compact Boundary\n- Historical `agentId`, `subRunId`, and `sessionId` values from \
         compacted history are non-authoritative.\n- Use the live direct-child snapshot or the \
         latest live tool result / child notification for routing.\n\n{}",
        summary.trim()
    )
}

fn summary_has_route_sensitive_content(summary: &str) -> bool {
    direct_child_validation_regex().is_match(summary)
        || child_agent_reference_block_regex().is_match(summary)
        || exact_agent_instruction_regex().is_match(summary)
        || raw_root_agent_id_regex().is_match(summary)
        || raw_agent_id_regex().is_match(summary)
        || raw_subrun_id_regex().is_match(summary)
        || raw_session_id_regex().is_match(summary)
        || [
            route_key_regex("agentId"),
            route_key_regex("childAgentId"),
            route_key_regex("parentAgentId"),
            route_key_regex("subRunId"),
            route_key_regex("parentSubRunId"),
            route_key_regex("sessionId"),
            route_key_regex("childSessionId"),
            route_key_regex("openSessionId"),
        ]
        .into_iter()
        .any(|regex| regex.is_match(summary))
}

fn child_agent_reference_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)Child agent reference:\s*(?:\n- .*)+")
            .expect("child agent reference regex should compile")
    })
}

fn direct_child_validation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)not a direct child of caller[^\n]*")
            .expect("direct child validation regex should compile")
    })
}

fn route_key_regex(key: &str) -> &'static Regex {
    static AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static CHILD_AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static PARENT_AGENT_ID: OnceLock<Regex> = OnceLock::new();
    static SUB_RUN_ID: OnceLock<Regex> = OnceLock::new();
    static PARENT_SUB_RUN_ID: OnceLock<Regex> = OnceLock::new();
    static SESSION_ID: OnceLock<Regex> = OnceLock::new();
    static CHILD_SESSION_ID: OnceLock<Regex> = OnceLock::new();
    static OPEN_SESSION_ID: OnceLock<Regex> = OnceLock::new();
    let slot = match key {
        "agentId" => &AGENT_ID,
        "childAgentId" => &CHILD_AGENT_ID,
        "parentAgentId" => &PARENT_AGENT_ID,
        "subRunId" => &SUB_RUN_ID,
        "parentSubRunId" => &PARENT_SUB_RUN_ID,
        "sessionId" => &SESSION_ID,
        "childSessionId" => &CHILD_SESSION_ID,
        "openSessionId" => &OPEN_SESSION_ID,
        other => panic!("unsupported route key regex: {other}"),
    };
    slot.get_or_init(|| {
        Regex::new(&format!(
            r"(?i)(?P<key>`?{key}`?\s*[:=]\s*`?)[^`\s,;\])]+`?"
        ))
        .expect("route key regex should compile")
    })
}

fn exact_agent_instruction_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(use this exact `agentid` value[^\n]*|copy it byte-for-byte[^\n]*|keep `agentid` exact[^\n]*)",
        )
        .expect("exact agent instruction regex should compile")
    })
}

fn raw_root_agent_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\broot-agent:[A-Za-z0-9._:-]+\b")
            .expect("raw root agent id regex should compile")
    })
}

fn raw_agent_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bagent-[A-Za-z0-9._:-]+\b").expect("raw agent id regex should compile")
    })
}

fn raw_subrun_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bsubrun-[A-Za-z0-9._:-]+\b").expect("raw subrun regex should compile")
    })
}

fn raw_session_id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\bsession-[A-Za-z0-9._:-]+\b").expect("raw session regex should compile")
    })
}

pub(super) fn strip_child_agent_reference_hint(content: &str) -> String {
    let Some((prefix, child_ref_block)) = content.split_once("\n\nChild agent reference:") else {
        return content.to_string();
    };
    let mut has_reference_fields = false;
    for line in child_ref_block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- agentId:")
            || trimmed.starts_with("- subRunId:")
            || trimmed.starts_with("- openSessionId:")
            || trimmed.starts_with("- status:")
        {
            has_reference_fields = true;
        }
    }
    let child_ref_summary = if has_reference_fields {
        "Child agent reference existed in the original tool result. Do not reuse any agentId, \
         subRunId, or sessionId from compacted history; rely on the latest live tool result or \
         current direct-child snapshot instead."
            .to_string()
    } else {
        "Child agent reference metadata existed in the original tool result, but compacted history \
         is not an authoritative source for later agent routing."
            .to_string()
    };
    let prefix = prefix.trim();
    if prefix.is_empty() {
        child_ref_summary
    } else {
        format!("{prefix}\n\n{child_ref_summary}")
    }
}
