use std::path::Path;

use crate::ComposerSkillSummary;

/// `App` 依赖的 skill 补全端口。
///
/// Why: composer 输入补全需要看到当前会话可见的 skill，
/// 但应用层不应直接依赖 `adapter-skills` 的实现细节。
pub trait ComposerSkillPort: Send + Sync {
    fn list_skill_summaries(&self, working_dir: &Path) -> Vec<ComposerSkillSummary>;
}
