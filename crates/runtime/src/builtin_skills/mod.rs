use std::fs;
use std::path::{Path, PathBuf};

use astrcode_core::home::resolve_home_dir;
use log::warn;

use crate::prompt::{parse_skill_md, SkillSource, SkillSpec};

struct BundledSkillDefinition {
    id: &'static str,
    assets: &'static [BundledSkillAsset],
    allowed_tools: &'static [&'static str],
    expand_tool_guides: bool,
}

struct BundledSkillAsset {
    relative_path: &'static str,
    content: &'static str,
}

const BUNDLED_SKILLS: &[BundledSkillDefinition] = &[];

pub(crate) fn builtin_skills() -> Vec<SkillSpec> {
    BUNDLED_SKILLS
        .iter()
        .map(|definition| {
            // Bundled skills are shipped with the binary, so an invalid SKILL.md
            // is a build-time authoring bug rather than optional user content.
            let skill_markdown = definition
                .assets
                .iter()
                .find(|asset| asset.relative_path == "SKILL.md")
                .unwrap_or_else(|| panic!("bundled skill '{}' is missing SKILL.md", definition.id))
                .content;
            let mut skill = parse_skill_md(skill_markdown, definition.id, SkillSource::Builtin)
                .unwrap_or_else(|| panic!("invalid bundled skill '{}'", definition.id));
            assert_valid_builtin_skill_identity(definition.id, &skill);
            // Keep Claude-style SKILL.md files focused on invocation guidance.
            // Execution metadata still lives in code so the file format stays
            // migratable from external skill repos without extra Astrcode keys.
            skill.allowed_tools = definition
                .allowed_tools
                .iter()
                .map(|tool| (*tool).to_string())
                .collect();
            skill.expand_tool_guides = definition.expand_tool_guides;
            if let Some(skill_root) = materialize_builtin_skill_assets(definition) {
                skill.reference_files = collect_reference_files(&skill_root);
                skill.skill_root = Some(skill_root.to_string_lossy().into_owned());
            }
            skill
        })
        .collect()
}

fn assert_valid_builtin_skill_identity(expected_id: &str, skill: &SkillSpec) {
    assert_eq!(
        skill.name, expected_id,
        "bundled skill frontmatter name must match its kebab-case folder name"
    );
    assert!(
        skill
            .name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
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
        // Materialize bundled assets onto disk so Claude-style references/ docs
        // can be discovered and opened with the same file tools as user skills.
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

fn write_asset_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content)
}

fn collect_reference_files(skill_root: &Path) -> Vec<String> {
    let references_dir = skill_root.join("references");
    let mut files = Vec::new();
    collect_files_recursive(&references_dir, skill_root, &mut files);
    files.sort();
    files
}

fn collect_files_recursive(root: &Path, base_dir: &Path, files: &mut Vec<String>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_files_recursive(&path, base_dir, files);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(base_dir) {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrcode_core::test_support::TestEnvGuard;

    #[test]
    fn bundled_skills_parse_from_claude_style_skill_md_assets() {
        let _guard = TestEnvGuard::new();
        let skills = builtin_skills();

        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn bundled_skills_materialize_claude_style_directory_layout() {
        // No bundled skills remain; the git-commit skill is loaded from the
        // user-level skills directory instead of the compiled-in bundle.
    }
}
