//! Composer 输入补全的 skill 查询端口。
//!
//! 定义 `ComposerSkillPort` trait 和 `ComposerResolvedSkill` 类型，
//! 将 composer 输入补全与 adapter-skills 的实现细节解耦。
//! 应用层不应直接依赖 `adapter-skills`，而是通过此端口获取当前会话可见的 skill 信息。

use std::path::Path;

use crate::ComposerSkillSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerResolvedSkill {
    pub id: String,
    pub description: String,
    pub guide: String,
}

/// `App` 依赖的 skill 补全端口。
///
/// Why: composer 输入补全需要看到当前会话可见的 skill，
/// 但应用层不应直接依赖 `adapter-skills` 的实现细节。
pub trait ComposerSkillPort: Send + Sync {
    fn list_skill_summaries(&self, working_dir: &Path) -> Vec<ComposerSkillSummary>;
    fn resolve_skill(&self, working_dir: &Path, skill_id: &str) -> Option<ComposerResolvedSkill>;
}
