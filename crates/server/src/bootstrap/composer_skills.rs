use std::{path::Path, sync::Arc};

use astrcode_application::{ComposerResolvedSkill, ComposerSkillPort, ComposerSkillSummary};
use astrcode_core::SkillCatalog;

#[derive(Clone)]
pub(crate) struct RuntimeComposerSkillPort {
    skill_catalog: Arc<dyn SkillCatalog>,
}

impl RuntimeComposerSkillPort {
    pub(crate) fn new(skill_catalog: Arc<dyn SkillCatalog>) -> Self {
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

    fn resolve_skill(&self, working_dir: &Path, skill_id: &str) -> Option<ComposerResolvedSkill> {
        self.skill_catalog
            .resolve_for_working_dir(&working_dir.to_string_lossy())
            .into_iter()
            .find(|skill| skill.matches_requested_name(skill_id))
            .map(|skill| ComposerResolvedSkill {
                id: skill.id,
                description: skill.description,
                guide: skill.guide,
            })
    }
}
