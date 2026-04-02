use std::fs;
use std::path::{Component, Path, PathBuf};

use astrcode_core::home::resolve_home_dir;
use log::warn;

use crate::{
    collect_asset_files, is_valid_skill_name, parse_skill_md, SkillSource, SkillSpec,
    SKILL_FILE_NAME,
};

struct BundledSkillDefinition {
    id: &'static str,
    assets: &'static [BundledSkillAsset],
}

struct BundledSkillAsset {
    relative_path: &'static str,
    content: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/bundled_skills.generated.rs"));

pub fn load_builtin_skills() -> Vec<SkillSpec> {
    BUNDLED_SKILLS
        .iter()
        .map(|definition| {
            // Bundled skills are authored inside this crate, so malformed
            // markdown should fail fast instead of silently disappearing.
            let skill_markdown = definition
                .assets
                .iter()
                .find(|asset| asset.relative_path == SKILL_FILE_NAME)
                .unwrap_or_else(|| panic!("bundled skill '{}' is missing SKILL.md", definition.id))
                .content;
            let mut skill = parse_skill_md(skill_markdown, definition.id, SkillSource::Builtin)
                .unwrap_or_else(|| panic!("invalid bundled skill '{}'", definition.id));
            assert_valid_builtin_skill_identity(definition.id, &skill);
            skill.allowed_tools = bundled_skill_allowed_tools(definition.id)
                .iter()
                .map(|tool| (*tool).to_string())
                .collect();
            if let Some(skill_root) = materialize_builtin_skill_assets(definition) {
                skill.asset_files = collect_asset_files(&skill_root);
                skill.skill_root = Some(skill_root.to_string_lossy().into_owned());
            }
            skill
        })
        .collect()
}

fn bundled_skill_allowed_tools(skill_id: &str) -> &'static [&'static str] {
    match skill_id {
        // The skill contract stays Claude-compatible in markdown, while runtime
        // records the actual tool boundary here for the Skill capability output.
        "git-commit" => &["shell", "readFile", "grep", "findFiles", "listDir"],
        _ => &[],
    }
}

fn assert_valid_builtin_skill_identity(expected_id: &str, skill: &SkillSpec) {
    assert_eq!(
        skill.name, expected_id,
        "bundled skill frontmatter name must match its kebab-case folder name"
    );
    assert!(
        is_valid_skill_name(&skill.name),
        "bundled skill names may only contain lowercase ascii letters, digits, and hyphens"
    );
}

fn materialize_builtin_skill_assets(definition: &BundledSkillDefinition) -> Option<PathBuf> {
    let home_dir = match resolve_home_dir() {
        Ok(home_dir) => home_dir,
        Err(error) => {
            warn!(
                "failed to resolve home directory for builtin skill '{}': {}",
                definition.id, error
            );
            return None;
        }
    };

    let skill_root = home_dir
        .join(".astrcode")
        .join("runtime")
        .join("builtin-skills")
        .join(definition.id);

    for asset in definition.assets {
        if !is_safe_relative_asset_path(asset.relative_path) {
            warn!(
                "skipping unsafe builtin skill asset '{}' for '{}'",
                asset.relative_path, definition.id
            );
            return None;
        }

        let asset_path = skill_root.join(
            asset
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if let Some(parent) = asset_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                warn!(
                    "failed to create builtin skill directory '{}' for '{}': {}",
                    parent.display(),
                    definition.id,
                    error
                );
                return None;
            }
        }

        // Materialize the bundled tree so `scripts/` and `references/` stay
        // executable/readable at runtime instead of living only in prompt text.
        if let Err(error) = write_asset_if_changed(&asset_path, asset.content) {
            warn!(
                "failed to materialize builtin skill asset '{}' for '{}': {}",
                asset.relative_path, definition.id, error
            );
            return None;
        }
    }

    Some(skill_root)
}

fn is_safe_relative_asset_path(relative_path: &str) -> bool {
    let path = Path::new(relative_path);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_)) || matches!(component, Component::CurDir)
        })
}

fn write_asset_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use astrcode_core::test_support::TestEnvGuard;

    use super::*;

    #[test]
    fn bundled_skills_parse_from_claude_style_skill_directories() {
        let _guard = TestEnvGuard::new();
        let skills = load_builtin_skills();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "git-commit");
    }

    #[test]
    fn bundled_skills_materialize_directory_assets() {
        let _guard = TestEnvGuard::new();
        let skills = load_builtin_skills();

        let skill_root = skills[0]
            .skill_root
            .as_ref()
            .expect("builtin skill root should be materialized");
        assert!(Path::new(skill_root).join("SKILL.md").is_file());
    }
}
