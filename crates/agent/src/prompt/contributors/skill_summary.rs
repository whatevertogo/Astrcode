use crate::prompt::{PromptContext, PromptContribution, PromptContributor};

pub struct SkillSummaryContributor;

impl PromptContributor for SkillSummaryContributor {
    fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        // TODO: add skill summaries and progressive tool disclosure here.
        PromptContribution::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_returns_empty_contribution() {
        let contributor = SkillSummaryContributor;
        let contribution = contributor.contribute(&PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
            step_index: 0,
            turn_index: 0,
        });

        assert!(contribution.system_blocks.is_empty());
        assert!(contribution.prepend_messages.is_empty());
        assert!(contribution.append_messages.is_empty());
        assert!(contribution.extra_tools.is_empty());
    }
}
