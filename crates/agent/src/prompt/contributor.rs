use super::{PromptContext, PromptContribution};

pub trait PromptContributor: Send + Sync {
    fn contribute(&self, ctx: &PromptContext) -> PromptContribution;
}
