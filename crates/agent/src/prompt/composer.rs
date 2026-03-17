use std::sync::Arc;

use super::contributors::{
    AgentsMdContributor, EnvironmentContributor, IdentityContributor, SkillSummaryContributor,
};
use super::{PromptContext, PromptContributor, PromptPlan};

pub struct PromptComposer {
    contributors: Vec<Arc<dyn PromptContributor>>,
}

impl PromptComposer {
    pub fn with_defaults() -> Self {
        Self {
            contributors: vec![
                Arc::new(IdentityContributor),
                Arc::new(EnvironmentContributor),
                Arc::new(AgentsMdContributor),
                Arc::new(SkillSummaryContributor),
            ],
        }
    }

    pub fn add(mut self, contributor: Arc<dyn PromptContributor>) -> Self {
        self.contributors.push(contributor);
        self
    }

    pub fn build(&self, ctx: &PromptContext) -> PromptPlan {
        let mut plan = PromptPlan::default();

        for contributor in &self.contributors {
            let contribution = contributor.contribute(ctx);
            plan.merge(contribution);
        }

        plan
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::prompt::{BlockKind, PromptBlock, PromptContribution};
    use crate::test_support::TestEnvGuard;

    fn test_context(working_dir: String) -> PromptContext {
        PromptContext {
            working_dir,
            tool_names: vec!["shell".to_string()],
            step_index: 0,
            turn_index: 0,
        }
    }

    struct StaticContributor;

    impl PromptContributor for StaticContributor {
        fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
            PromptContribution {
                system_blocks: vec![PromptBlock {
                    kind: BlockKind::Skill,
                    title: "Skill",
                    content: "static".to_string(),
                }],
                ..PromptContribution::default()
            }
        }
    }

    struct CountingContributor {
        calls: Arc<AtomicUsize>,
    }

    impl PromptContributor for CountingContributor {
        fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
            self.calls.fetch_add(1, Ordering::SeqCst);
            PromptContribution::default()
        }
    }

    #[test]
    fn with_defaults_build_includes_identity_block() {
        let guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let composer = PromptComposer::with_defaults();

        let plan = composer.build(&test_context(project.path().to_string_lossy().into_owned()));

        assert!(plan
            .system_blocks
            .iter()
            .any(|block| block.kind == BlockKind::Identity));
        assert!(!guard
            .home_dir()
            .join(".astrcode")
            .join("AGENTS.md")
            .exists());
    }

    #[test]
    fn add_appends_custom_contributor_output() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let composer = PromptComposer::with_defaults().add(Arc::new(StaticContributor));

        let plan = composer.build(&test_context(project.path().to_string_lossy().into_owned()));

        assert!(plan
            .system_blocks
            .iter()
            .any(|block| block.kind == BlockKind::Skill && block.content == "static"));
    }

    #[test]
    fn build_calls_each_contributor_once() {
        let _guard = TestEnvGuard::new();
        let project = tempfile::tempdir().expect("tempdir should be created");
        let calls = Arc::new(AtomicUsize::new(0));
        let composer = PromptComposer::with_defaults().add(Arc::new(CountingContributor {
            calls: calls.clone(),
        }));

        let _plan = composer.build(&test_context(project.path().to_string_lossy().into_owned()));

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
