//! Skill 目录 (Skill Catalog)
//!
//! 统一管理所有来源的 skill，提供唯一的解析入口。
//!
//! ## 架构设计
//!
//! `SkillCatalog` 持有 **base skills**（builtin + plugin + mcp），
//! 并通过 `resolve_for_working_dir()` 动态合并 user 和 project skill。
//!
//! ## 覆盖优先级
//!
//! 同名 skill 按以下顺序覆盖（后者覆盖前者）：
//! `builtin < mcp < plugin < user < project`
//!
//! 注意：base skills 内部已经按 `builtin < mcp < plugin` 排序，
//! `resolve_for_working_dir` 再在其上叠加 `user` 和 `project`。
//!
//! ## 线程安全
//!
//! `SkillCatalog` 使用 `Arc<RwLock<>>` 包装，支持并发读取和原子替换。
//! Runtime reload 时，新的 base skills 会原子地替换旧的。

use std::sync::{Arc, RwLock};

use log::debug;

use crate::skill_loader::{load_project_skills, load_user_skills};
use crate::SkillSpec;

/// Skill 目录，持有 base skills 并提供统一的解析入口。
///
/// Base skills 包含 builtin、plugin、mcp 来源的 skill，
/// 在 runtime 装配时一次性构建。User 和 project skill 在每次解析时动态加载。
#[derive(Debug, Clone)]
pub struct SkillCatalog {
    /// Base skills（builtin + plugin + mcp），按优先级排序。
    /// 使用 RwLock 支持并发读取和原子替换。
    base_skills: Arc<RwLock<Vec<SkillSpec>>>,
}

impl SkillCatalog {
    /// 创建新的 SkillCatalog。
    ///
    /// `base_skills` 应按优先级从低到高排序（builtin < mcp < plugin），
    /// 这样后续的覆盖逻辑才能正确工作。
    pub fn new(base_skills: Vec<SkillSpec>) -> Self {
        Self {
            base_skills: Arc::new(RwLock::new(normalize_base_skills(base_skills))),
        }
    }

    /// 原子替换 base skills。
    ///
    /// 用于 runtime reload 场景，新的 base skills 会完全替换旧的。
    /// 调用方应确保 `new_base_skills` 已按优先级排序。
    pub fn replace_base_skills(&self, new_base_skills: Vec<SkillSpec>) {
        let mut guard = self.base_skills.write().unwrap();
        *guard = normalize_base_skills(new_base_skills);
    }

    /// 获取当前 base skills 的快照。
    pub fn base_skills(&self) -> Vec<SkillSpec> {
        let guard = self.base_skills.read().unwrap();
        guard.clone()
    }

    /// 解析指定工作目录下的完整 skill 列表。
    ///
    /// 合并 base skills（builtin + plugin + mcp）、user skills 和 project skills。
    /// 覆盖优先级：`builtin < mcp < plugin < user < project`
    ///
    /// 注意：base skills 内部已经按 `builtin < mcp < plugin` 排序，
    /// 此方法在其基础上叠加 user 和 project skill。
    pub fn resolve_for_working_dir(&self, working_dir: &str) -> Vec<SkillSpec> {
        let base = self.base_skills();
        resolve_skills(&base, working_dir)
    }
}

/// 合并 base skills、user skills 和 project skills。
///
/// 这是 `SkillCatalog::resolve_for_working_dir` 的核心逻辑。
///
/// 保持为 crate 内部函数，避免外部调用方绕过 `SkillCatalog`
/// 直接把 skill 解析重新分散到各处。
///
/// ## 覆盖优先级
///
/// `builtin < mcp < plugin < user < project`
///
/// 注意：`base_skills` 内部已经按 `builtin < mcp < plugin` 排序。
/// 此方法先叠加 user，再叠加 project，最终得到正确的优先级顺序。
pub(crate) fn resolve_skills(base_skills: &[SkillSpec], working_dir: &str) -> Vec<SkillSpec> {
    let with_user_skills = merge_skill_layers(base_skills.to_vec(), load_user_skills());
    merge_skill_layers(with_user_skills, load_project_skills(working_dir))
}

/// 合并两层 skill 列表，后者覆盖前者。
///
/// 同名 skill（按 `id` 匹配）以 `overrides` 中的版本为准。
/// 这是实现 skill 覆盖优先级的核心逻辑。
///
/// 当发生覆盖时，会记录调试日志，标明 winner/loser/source。
pub fn merge_skill_layers(mut base: Vec<SkillSpec>, overrides: Vec<SkillSpec>) -> Vec<SkillSpec> {
    for skill in overrides {
        if let Some(existing) = base.iter_mut().find(|candidate| candidate.id == skill.id) {
            // 同名覆盖是正常的优先级行为，因此只记调试日志，避免把预期覆盖误报成 warning。
            debug!(
                "skill '{}' overridden: winner=source:{}, loser=source:{}",
                skill.id,
                skill.source.as_tag(),
                existing.source.as_tag()
            );
            *existing = skill;
        } else {
            base.push(skill);
        }
    }
    base
}

fn normalize_base_skills(base_skills: Vec<SkillSpec>) -> Vec<SkillSpec> {
    base_skills.into_iter().fold(Vec::new(), |base, skill| {
        merge_skill_layers(base, vec![skill])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SkillSource;

    fn make_skill(id: &str, source: SkillSource) -> SkillSpec {
        SkillSpec {
            id: id.to_string(),
            name: id.to_string(),
            description: format!("desc for {}", id),
            guide: format!("guide for {}", id),
            skill_root: None,
            asset_files: vec![],
            allowed_tools: vec![],
            source,
        }
    }

    #[test]
    fn test_merge_layers_override() {
        let builtin = vec![make_skill("git-commit", SkillSource::Builtin)];
        let user = vec![make_skill("git-commit", SkillSource::User)];
        let merged = merge_skill_layers(builtin, user);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, SkillSource::User);
    }

    #[test]
    fn test_merge_layers_add_new() {
        let builtin = vec![make_skill("git-commit", SkillSource::Builtin)];
        let user = vec![make_skill("repo-search", SkillSource::User)];
        let merged = merge_skill_layers(builtin, user);

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_catalog_resolve_priority() {
        // Base: builtin < mcp < plugin
        let base = vec![
            make_skill("git-commit", SkillSource::Builtin),
            make_skill("git-commit", SkillSource::Mcp),
            make_skill("git-commit", SkillSource::Plugin),
        ];
        let catalog = SkillCatalog::new(base);
        // 这里只验证 base skill 的归一化顺序，避免测试受本机 user/project skill 目录污染。
        let normalized = catalog.base_skills();
        let git_skill = normalized.iter().find(|s| s.id == "git-commit");
        assert!(git_skill.is_some());
        assert_eq!(git_skill.unwrap().source, SkillSource::Plugin);
    }

    #[test]
    fn test_catalog_replace_base_skills() {
        let catalog = SkillCatalog::new(vec![make_skill("old-skill", SkillSource::Builtin)]);
        assert_eq!(catalog.base_skills().len(), 1);

        catalog.replace_base_skills(vec![
            make_skill("new-skill-1", SkillSource::Builtin),
            make_skill("new-skill-2", SkillSource::Plugin),
        ]);
        assert_eq!(catalog.base_skills().len(), 2);
    }
}
