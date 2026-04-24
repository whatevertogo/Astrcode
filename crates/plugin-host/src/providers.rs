use astrcode_core::Result;

use crate::{PluginDescriptor, ProviderDescriptor, descriptor::validate_descriptors};

pub const OPENAI_PROVIDER_ID: &str = "openai";
pub const OPENAI_API_KIND: &str = "openai";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderContributionCatalog {
    pub providers: Vec<ProviderDescriptor>,
}

pub fn builtin_openai_provider_descriptor() -> PluginDescriptor {
    let mut descriptor = PluginDescriptor::builtin("builtin-provider-openai", "Builtin OpenAI");
    descriptor.providers.push(ProviderDescriptor {
        provider_id: OPENAI_PROVIDER_ID.to_string(),
        api_kind: OPENAI_API_KIND.to_string(),
    });
    descriptor
}

impl ProviderContributionCatalog {
    pub fn from_descriptors(descriptors: &[PluginDescriptor]) -> Result<Self> {
        validate_descriptors(descriptors)?;
        Ok(Self {
            providers: descriptors
                .iter()
                .flat_map(|descriptor| descriptor.providers.iter().cloned())
                .collect(),
        })
    }

    pub fn provider(&self, provider_id: &str) -> Option<&ProviderDescriptor> {
        self.providers
            .iter()
            .find(|provider| provider.provider_id == provider_id)
    }

    pub fn provider_for_api_kind(&self, api_kind: &str) -> Option<&ProviderDescriptor> {
        self.providers
            .iter()
            .find(|provider| provider.api_kind == api_kind)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OPENAI_API_KIND, OPENAI_PROVIDER_ID, ProviderContributionCatalog,
        builtin_openai_provider_descriptor,
    };
    use crate::{PluginDescriptor, ProviderDescriptor};

    #[test]
    fn builtin_openai_provider_is_registered_through_descriptor() {
        let descriptor = builtin_openai_provider_descriptor();
        let catalog = ProviderContributionCatalog::from_descriptors(&[descriptor])
            .expect("provider catalog should build");

        let provider = catalog
            .provider(OPENAI_PROVIDER_ID)
            .expect("openai provider should exist");

        assert_eq!(provider.api_kind, OPENAI_API_KIND);
    }

    #[test]
    fn provider_catalog_accepts_plugin_provider_descriptors() {
        let mut descriptor = PluginDescriptor::builtin("corp-provider", "Corp Provider");
        descriptor.providers.push(ProviderDescriptor {
            provider_id: "corp-ai".to_string(),
            api_kind: "openai-compatible".to_string(),
        });

        let catalog = ProviderContributionCatalog::from_descriptors(&[descriptor])
            .expect("provider catalog should build");

        assert_eq!(
            catalog
                .provider_for_api_kind("openai-compatible")
                .expect("provider should be indexed")
                .provider_id,
            "corp-ai"
        );
    }

    #[test]
    fn provider_catalog_rejects_duplicate_provider_ids() {
        let mut first = PluginDescriptor::builtin("first", "First");
        first.providers.push(ProviderDescriptor {
            provider_id: "shared".to_string(),
            api_kind: "openai".to_string(),
        });
        let mut second = PluginDescriptor::builtin("second", "Second");
        second.providers.push(ProviderDescriptor {
            provider_id: "shared".to_string(),
            api_kind: "anthropic".to_string(),
        });

        let error = ProviderContributionCatalog::from_descriptors(&[first, second])
            .expect_err("duplicate provider ids should fail");

        assert!(error.to_string().contains("provider 'shared'"));
    }
}
