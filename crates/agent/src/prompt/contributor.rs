use async_trait::async_trait;

use super::{PromptContext, PromptContribution};

#[async_trait]
pub trait PromptContributor: Send + Sync {
    fn contributor_id(&self) -> &'static str;

    fn cache_version(&self) -> u64 {
        1
    }

    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        ctx.contributor_cache_fingerprint()
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution;
}
