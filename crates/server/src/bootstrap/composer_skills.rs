use std::{path::Path, sync::Arc};

use astrcode_adapter_skills::SkillCatalog;
use astrcode_application::{ComposerSkillPort, ComposerSkillSummary};

#[derive(Clone)]
pub(crate) struct RuntimeComposerSkillPort {
    skill_catalog: Arc<SkillCatalog>,
}

impl RuntimeComposerSkillPort {
    pub(crate) fn new(skill_catalog: Arc<SkillCatalog>) -> Self {
        Self { skill_catalog }
    }
}

impl std::fmt::Debug for RuntimeComposerSkillPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeComposerSkillPort")
            .finish_non_exhaustive()
    }
}

impl ComposerSkillPort for RuntimeComposerSkillPort {
    fn list_skill_summaries(&self, working_dir: &Path) -> Vec<ComposerSkillSummary> {
        self.skill_catalog
            .resolve_for_working_dir(&working_dir.to_string_lossy())
            .into_iter()
            .map(|skill| ComposerSkillSummary::new(skill.id, skill.description))
            .collect()
    }
}
