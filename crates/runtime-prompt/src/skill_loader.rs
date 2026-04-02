use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::home::resolve_home_dir;
use log::warn;
use serde::Deserialize;

use crate::contributors::cache_marker_for_path;
use crate::{SkillSource, SkillSpec};

const SKILL_FILE_NAME: &str = "SKILL.md";

/// Frontmatter fields stay optional so new metadata can roll out without
/// forcing older skill files to adopt every key immediately.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(alias = "whenToUse")]
    pub when_to_use: Option<String>,
}

pub fn parse_skill_md(content: &str, fallback_id: &str, source: SkillSource) -> Option<SkillSpec> {
    let normalized = normalize_skill_content(content);
    if normalized.trim().is_empty() {
        return None;
    }

    let (frontmatter, body) = match split_frontmatter(&normalized) {
        Some((frontmatter, body)) => match serde_yaml::from_str::<SkillFrontmatter>(frontmatter) {
            Ok(frontmatter) => (frontmatter, body),
            Err(error) => {
                warn!("failed to parse frontmatter for skill '{fallback_id}': {error}");
                return None;
            }
        },
        None => (SkillFrontmatter::default(), normalized.as_str()),
    };

    let guide = body.trim().to_string();
    if guide.is_empty() {
        return None;
    }

    let name = frontmatter
        .name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_id.to_string());
    let description = build_skill_description(frontmatter.description, frontmatter.when_to_use);

    Some(SkillSpec {
        // Directory names are stable machine identifiers; frontmatter names are
        // display labels and may change without meaning a different skill.
        id: fallback_id.to_string(),
        name,
        description,
        guide,
        required_tools: Vec::new(),
        triggers: Vec::new(),
        source,
        expand_tool_guides: false,
    })
}

pub fn load_skills_from_dir(dir: &Path, source: SkillSource) -> Vec<SkillSpec> {
    if !dir.exists() {
        return Vec::new();
    }

    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!("failed to read skill directory {}: {error}", dir.display());
            return Vec::new();
        }
    };

    let mut children = entries.filter_map(Result::ok).collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());

    let mut skills = Vec::new();
    for entry in children {
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warn!(
                    "failed to inspect skill directory entry {}: {error}",
                    entry.path().display()
                );
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }

        let skill_dir = entry.path();
        let skill_path = skill_dir.join(SKILL_FILE_NAME);
        if !skill_path.is_file() {
            continue;
        }

        let folder_name = entry.file_name();
        let fallback_id = folder_name.to_string_lossy();
        let content = match fs::read_to_string(&skill_path) {
            Ok(content) => content,
            Err(error) => {
                warn!("failed to read {}: {error}", skill_path.display());
                continue;
            }
        };

        if let Some(skill) = parse_skill_md(&content, &fallback_id, source.clone()) {
            skills.push(skill);
        }
    }

    skills
}

pub fn load_user_skills() -> Vec<SkillSpec> {
    let Some(home_dir) = resolve_user_home_dir() else {
        return Vec::new();
    };

    let claude_skills =
        load_skills_from_dir(&home_dir.join(".claude").join("skills"), SkillSource::User);
    let astrcode_skills = load_skills_from_dir(
        &home_dir.join(".astrcode").join("skills"),
        SkillSource::User,
    );

    merge_skill_layers(claude_skills, astrcode_skills)
}

pub fn load_project_skills(working_dir: &str) -> Vec<SkillSpec> {
    load_skills_from_dir(
        &PathBuf::from(working_dir).join(".astrcode").join("skills"),
        SkillSource::Project,
    )
}

pub fn resolve_prompt_skills(base_skills: &[SkillSpec], working_dir: &str) -> Vec<SkillSpec> {
    let with_user_skills = merge_skill_layers(base_skills.to_vec(), load_user_skills());
    merge_skill_layers(with_user_skills, load_project_skills(working_dir))
}

pub fn skill_roots_cache_marker(working_dir: &str) -> String {
    let mut markers = Vec::new();

    if let Some(home_dir) = resolve_user_home_dir() {
        markers.push(cache_marker_for_skill_root(
            &home_dir.join(".claude").join("skills"),
        ));
        markers.push(cache_marker_for_skill_root(
            &home_dir.join(".astrcode").join("skills"),
        ));
    } else {
        markers.push("user-home=<unresolved>".to_string());
    }

    markers.push(cache_marker_for_skill_root(
        &PathBuf::from(working_dir).join(".astrcode").join("skills"),
    ));

    markers.join("|")
}

fn normalize_skill_content(content: &str) -> String {
    content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    if !content.starts_with("---\n") {
        return None;
    }

    let rest = &content[4..];
    if let Some(end) = rest.find("\n---\n") {
        return Some((&rest[..end], &rest[end + 5..]));
    }

    rest.find("\n---")
        .filter(|end| rest[*end + 4..].is_empty())
        .map(|end| (&rest[..end], ""))
}

fn resolve_user_home_dir() -> Option<PathBuf> {
    match resolve_home_dir() {
        Ok(home_dir) => Some(home_dir),
        Err(error) => {
            warn!("failed to resolve home directory for skills: {error}");
            None
        }
    }
}

fn build_skill_description(description: Option<String>, when_to_use: Option<String>) -> String {
    match (description, when_to_use) {
        (Some(description), Some(when_to_use))
            if !description.trim().is_empty() && !when_to_use.trim().is_empty() =>
        {
            format!("{description}\nWhen to use: {when_to_use}")
        }
        (Some(description), _) if !description.trim().is_empty() => description,
        (_, Some(when_to_use)) if !when_to_use.trim().is_empty() => {
            format!("When to use: {when_to_use}")
        }
        _ => String::new(),
    }
}

fn merge_skill_layers(mut base: Vec<SkillSpec>, overrides: Vec<SkillSpec>) -> Vec<SkillSpec> {
    for skill in overrides {
        if let Some(existing) = base.iter_mut().find(|candidate| candidate.id == skill.id) {
            *existing = skill;
        } else {
            base.push(skill);
        }
    }

    base
}

fn cache_marker_for_skill_root(root: &Path) -> String {
    if !root.exists() {
        return format!("{}=missing", root.display());
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            warn!("failed to read skill directory {}: {error}", root.display());
            return format!("{}=unreadable", root.display());
        }
    };

    let mut markers = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let skill_path = entry.path().join(SKILL_FILE_NAME);
        markers.push(format!(
            "{}={}",
            skill_path.display(),
            cache_marker_for_path(&skill_path)
        ));
    }
    markers.sort();

    format!("{}:[{}]", root.display(), markers.join(","))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use astrcode_core::test_support::TestEnvGuard;

    use super::*;

    fn write_skill(root: &Path, name: &str, content: &str) {
        let skill_dir = root.join(name);
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(skill_dir.join(SKILL_FILE_NAME), content).expect("skill file should be written");
    }

    #[test]
    fn parse_skill_md_with_frontmatter() {
        let parsed = parse_skill_md(
            "---\nname: Git Commit\ndescription: Commit workflow\nwhen_to_use: When the user asks for commits\n---\n# Guide\nUse commit skill.\n",
            "git-commit",
            SkillSource::User,
        )
        .expect("frontmatter skill should parse");

        assert_eq!(parsed.id, "git-commit");
        assert_eq!(parsed.name, "Git Commit");
        assert_eq!(
            parsed.description,
            "Commit workflow\nWhen to use: When the user asks for commits"
        );
        assert_eq!(parsed.guide, "# Guide\nUse commit skill.");
        assert_eq!(parsed.source, SkillSource::User);
    }

    #[test]
    fn parse_skill_md_without_frontmatter() {
        let parsed = parse_skill_md(
            "# Guide\nUse grep first.",
            "repo-search",
            SkillSource::Project,
        )
        .expect("plain markdown should parse");

        assert_eq!(parsed.id, "repo-search");
        assert_eq!(parsed.name, "repo-search");
        assert_eq!(parsed.description, "");
        assert_eq!(parsed.guide, "# Guide\nUse grep first.");
    }

    #[test]
    fn parse_skill_md_empty_content() {
        assert!(parse_skill_md(" \n\t", "empty", SkillSource::User).is_none());
    }

    #[test]
    fn parse_skill_md_empty_guide() {
        assert!(parse_skill_md("---\nname: Empty\n---\n", "empty", SkillSource::User).is_none());
    }

    #[test]
    fn parse_skill_md_supports_bom_and_crlf() {
        let parsed = parse_skill_md(
            "\u{feff}---\r\nname: Windows\r\ndescription: CRLF\r\n---\r\nLine 1\r\nLine 2\r\n",
            "windows",
            SkillSource::User,
        )
        .expect("BOM + CRLF skill should parse");

        assert_eq!(parsed.name, "Windows");
        assert_eq!(parsed.guide, "Line 1\nLine 2");
    }

    #[test]
    fn parse_skill_md_invalid_frontmatter_is_skipped() {
        assert!(
            parse_skill_md("---\nname: [oops\n---\nbody", "broken", SkillSource::User).is_none()
        );
    }

    #[test]
    fn load_skills_from_dir_scans_subdirs() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        write_skill(dir.path(), "git-commit", "# Commit guide");
        write_skill(dir.path(), "repo-search", "# Search guide");

        let skills = load_skills_from_dir(dir.path(), SkillSource::User);
        let ids = skills.into_iter().map(|skill| skill.id).collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec!["git-commit".to_string(), "repo-search".to_string()]
        );
    }

    #[test]
    fn load_skills_from_dir_skips_non_skill_dirs() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        fs::create_dir_all(dir.path().join("empty")).expect("empty dir should be created");
        write_skill(dir.path(), "git-commit", "# Commit guide");

        let skills = load_skills_from_dir(dir.path(), SkillSource::User);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "git-commit");
    }

    #[test]
    fn load_skills_from_dir_nonexistent_dir() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let missing = dir.path().join("missing");

        assert!(load_skills_from_dir(&missing, SkillSource::User).is_empty());
    }

    #[test]
    fn resolve_prompt_skills_applies_expected_precedence() {
        let guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");

        write_skill(
            &guard.home_dir().join(".claude").join("skills"),
            "shared",
            "---\nname: Claude skill\n---\nClaude guide",
        );
        write_skill(
            &guard.home_dir().join(".astrcode").join("skills"),
            "shared",
            "---\nname: Astrcode skill\n---\nAstrcode guide",
        );
        write_skill(
            &project.path().join(".astrcode").join("skills"),
            "shared",
            "---\nname: Project skill\n---\nProject guide",
        );

        let resolved = resolve_prompt_skills(
            &[SkillSpec {
                id: "shared".to_string(),
                name: "Builtin skill".to_string(),
                description: "builtin".to_string(),
                guide: "Builtin guide".to_string(),
                required_tools: Vec::new(),
                triggers: Vec::new(),
                source: SkillSource::Builtin,
                expand_tool_guides: false,
            }],
            &project.path().to_string_lossy(),
        );

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "Project skill");
        assert_eq!(resolved[0].guide, "Project guide");
        assert_eq!(resolved[0].source, SkillSource::Project);
    }

    #[test]
    fn skill_roots_cache_marker_changes_when_project_skill_is_added() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let working_dir = project.path().to_string_lossy().into_owned();

        let before = skill_roots_cache_marker(&working_dir);
        write_skill(
            &project.path().join(".astrcode").join("skills"),
            "project-skill",
            "# Project guide",
        );
        let after = skill_roots_cache_marker(&working_dir);

        assert_ne!(before, after);
    }
}
