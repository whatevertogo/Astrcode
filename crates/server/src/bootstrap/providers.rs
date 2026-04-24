//! # Provider 装配
//!
//! 负责选择具体 adapter 实现与配置驱动策略，
//! 让组合根只表达依赖关系，不再旁路 application 已有的配置模型。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use astrcode_adapter_agents::AgentProfileLoader;
use astrcode_adapter_llm::{
    LlmClientConfig, ModelLimits,
    openai::{OpenAiProvider, OpenAiProviderCapabilities},
};
use astrcode_adapter_storage::config_store::FileConfigStore;
use astrcode_agent_runtime::{LlmEventSink, LlmOutput, LlmProvider, LlmRequest};
use astrcode_core::config::{OpenAiApiMode, OpenAiProfileCapabilities};
use astrcode_plugin_host::{OPENAI_API_KIND, ProviderContributionCatalog};

use super::deps::core::{
    AgentProfile, AstrError, ModelConfig, ResolvedRuntimeConfig, Result, resolve_runtime_config,
};
use crate::{
    ApplicationError, ConfigService, ProfileProvider, ProfileResolutionService,
    application_error_bridge::ServerRouteError,
    config_mode_helpers,
    config_service_bridge::ServerConfigService,
    profile_service::{ServerProfilePort, ServerProfileService},
};

pub(crate) fn build_llm_provider(
    config_service: Arc<ServerConfigService>,
    working_dir: PathBuf,
    provider_catalog: Arc<RwLock<ProviderContributionCatalog>>,
) -> Arc<dyn LlmProvider> {
    Arc::new(ConfigBackedLlmProvider::new(
        config_service,
        working_dir,
        provider_catalog,
    ))
}

pub(crate) fn build_config_service(config_path: PathBuf) -> Result<Arc<ServerConfigService>> {
    let config_store = FileConfigStore::new(config_path);
    Ok(Arc::new(ServerConfigService::new(Arc::new(
        ConfigService::new(Arc::new(config_store)),
    ))))
}

pub(crate) fn build_profile_resolution_service(
    loader: AgentProfileLoader,
) -> Result<Arc<ServerProfileService>> {
    let provider: Arc<dyn ProfileProvider> = Arc::new(LoaderBackedProfileProvider { loader });
    let profile_resolver = Arc::new(ProfileResolutionService::new(provider));
    Ok(Arc::new(ServerProfileService::new(Arc::new(
        ApplicationProfilePort {
            inner: Arc::clone(&profile_resolver),
        },
    ))))
}

struct ConfigBackedLlmProvider {
    config_service: Arc<ServerConfigService>,
    working_dir: PathBuf,
    provider_catalog: Arc<RwLock<ProviderContributionCatalog>>,
    providers: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
}

struct LoaderBackedProfileProvider {
    loader: AgentProfileLoader,
}

struct ApplicationProfilePort {
    inner: Arc<ProfileResolutionService>,
}

impl ServerProfilePort for ApplicationProfilePort {
    fn resolve(
        &self,
        working_dir: &Path,
    ) -> std::result::Result<Arc<Vec<AgentProfile>>, ServerRouteError> {
        self.inner
            .resolve(working_dir)
            .map_err(application_error_to_server)
    }

    fn find_profile(
        &self,
        working_dir: &Path,
        profile_id: &str,
    ) -> std::result::Result<AgentProfile, ServerRouteError> {
        self.inner
            .find_profile(working_dir, profile_id)
            .map_err(application_error_to_server)
    }

    fn resolve_global(&self) -> std::result::Result<Arc<Vec<AgentProfile>>, ServerRouteError> {
        self.inner
            .resolve_global()
            .map_err(application_error_to_server)
    }

    fn invalidate(&self, working_dir: &Path) {
        self.inner.invalidate(working_dir);
    }

    fn invalidate_global(&self) {
        self.inner.invalidate_global();
    }

    fn invalidate_all(&self) {
        self.inner.invalidate_all();
    }
}

impl ProfileProvider for LoaderBackedProfileProvider {
    fn load_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> std::result::Result<Vec<AgentProfile>, ApplicationError> {
        let registry = self
            .loader
            .load_for_working_dir(Some(working_dir))
            .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        Ok(registry.list().into_iter().cloned().collect())
    }

    fn load_global(&self) -> std::result::Result<Vec<AgentProfile>, ApplicationError> {
        let registry = self
            .loader
            .load()
            .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        Ok(registry.list().into_iter().cloned().collect())
    }
}

impl std::fmt::Debug for ConfigBackedLlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigBackedLlmProvider")
            .field("working_dir", &self.working_dir)
            .finish_non_exhaustive()
    }
}

impl ConfigBackedLlmProvider {
    fn new(
        config_service: Arc<ServerConfigService>,
        working_dir: PathBuf,
        provider_catalog: Arc<RwLock<ProviderContributionCatalog>>,
    ) -> Self {
        Self {
            config_service,
            working_dir,
            provider_catalog,
            providers: RwLock::new(HashMap::new()),
        }
    }

    fn resolve_spec(&self) -> std::result::Result<ResolvedLlmProviderSpec, ApplicationError> {
        let config = self
            .config_service
            .load_overlayed_config(Some(self.working_dir.as_path()))
            .map_err(server_error_to_application)?;
        let selection = config_mode_helpers::resolve_current_model(&config)
            .map_err(|error| ApplicationError::InvalidArgument(error.to_string()))?;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == selection.profile_name)
            .ok_or_else(|| {
                ApplicationError::InvalidArgument(format!(
                    "profile '{}' not found in resolved config",
                    selection.profile_name
                ))
            })?;
        let model = profile
            .models
            .iter()
            .find(|model| model.id == selection.model)
            .ok_or_else(|| {
                ApplicationError::InvalidArgument(format!(
                    "model '{}' not found under profile '{}'",
                    selection.model, profile.name
                ))
            })?;
        let api_key = config_mode_helpers::resolve_api_key(profile)
            .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        let limits = resolve_model_limits(&profile.provider_kind, model);
        let runtime = resolve_runtime_config(&config.runtime);
        let client_config = client_config_from_runtime(&runtime);
        let provider_descriptor = {
            let catalog = self
                .provider_catalog
                .read()
                .expect("provider catalog read lock poisoned");
            catalog
                .provider(&profile.provider_kind)
                .or_else(|| catalog.provider_for_api_kind(&profile.provider_kind))
                .cloned()
                .ok_or_else(|| {
                    ApplicationError::InvalidArgument(format!(
                        "unsupported provider_kind '{}'：未在 plugin-host ProviderDescriptor \
                         catalog 中注册",
                        profile.provider_kind
                    ))
                })?
        };
        if provider_descriptor.api_kind != OPENAI_API_KIND {
            return Err(ApplicationError::InvalidArgument(format!(
                "registered provider '{}' uses unsupported api_kind '{}'",
                provider_descriptor.provider_id, provider_descriptor.api_kind
            )));
        }
        let api_mode = resolve_openai_api_mode(profile);
        let endpoint = match api_mode {
            OpenAiApiMode::ChatCompletions => {
                config_mode_helpers::resolve_openai_chat_completions_api_url(&profile.base_url)
            },
            OpenAiApiMode::Responses => {
                config_mode_helpers::resolve_openai_responses_api_url(&profile.base_url)
            },
        };
        let openai_capabilities = Some(resolve_openai_provider_capabilities(
            endpoint.as_str(),
            profile.openai_capabilities.as_ref(),
        ));

        Ok(ResolvedLlmProviderSpec {
            cache_key: format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                provider_descriptor.provider_id,
                provider_descriptor.api_kind,
                match api_mode {
                    OpenAiApiMode::ChatCompletions => "chat_completions",
                    OpenAiApiMode::Responses => "responses",
                },
                endpoint,
                profile.name,
                model.id,
                client_config.connect_timeout.as_secs(),
                client_config.read_timeout.as_secs(),
                client_config.max_retries,
                client_config.retry_base_delay.as_millis(),
                openai_capabilities
                    .map(|caps| caps.supports_prompt_cache_key)
                    .unwrap_or(false),
                openai_capabilities
                    .map(|caps| caps.supports_stream_usage)
                    .unwrap_or(false)
            ),
            provider_id: provider_descriptor.provider_id,
            api_kind: provider_descriptor.api_kind,
            endpoint,
            api_key,
            model: model.id.clone(),
            limits,
            client_config,
            openai_capabilities,
        })
    }

    fn resolve_runtime_provider(&self) -> Result<Arc<dyn LlmProvider>> {
        let spec = self
            .resolve_spec()
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        if let Some(existing) = self
            .providers
            .read()
            .expect("llm provider cache read lock")
            .get(&spec.cache_key)
            .cloned()
        {
            return Ok(existing);
        }

        if spec.api_kind != OPENAI_API_KIND {
            return Err(AstrError::Validation(format!(
                "registered provider '{}' uses unsupported api_kind '{}'",
                spec.provider_id, spec.api_kind
            )));
        }
        let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiProvider::new_with_capabilities(
            spec.endpoint.clone(),
            spec.api_key.clone(),
            spec.model.clone(),
            spec.limits,
            spec.client_config,
            spec.openai_capabilities.unwrap_or_else(|| {
                OpenAiProviderCapabilities::for_endpoint(spec.endpoint.as_str())
            }),
        )?);

        self.providers
            .write()
            .expect("llm provider cache write lock")
            .insert(spec.cache_key, provider.clone());
        Ok(provider)
    }
}

fn server_error_to_application(error: ServerRouteError) -> ApplicationError {
    match error {
        ServerRouteError::NotFound(message) => ApplicationError::NotFound(message),
        ServerRouteError::Conflict(message) => ApplicationError::Conflict(message),
        ServerRouteError::InvalidArgument(message) => ApplicationError::InvalidArgument(message),
        ServerRouteError::PermissionDenied(message) => ApplicationError::PermissionDenied(message),
        ServerRouteError::Internal(message) => ApplicationError::Internal(message),
    }
}

fn application_error_to_server(error: ApplicationError) -> ServerRouteError {
    match error {
        ApplicationError::NotFound(message) => ServerRouteError::NotFound(message),
        ApplicationError::Conflict(message) => ServerRouteError::Conflict(message),
        ApplicationError::InvalidArgument(message) => ServerRouteError::InvalidArgument(message),
        ApplicationError::PermissionDenied(message) => ServerRouteError::PermissionDenied(message),
        ApplicationError::Internal(message) => ServerRouteError::Internal(message),
    }
}

fn client_config_from_runtime(runtime: &ResolvedRuntimeConfig) -> LlmClientConfig {
    LlmClientConfig {
        connect_timeout: std::time::Duration::from_secs(runtime.llm_connect_timeout_secs),
        read_timeout: std::time::Duration::from_secs(runtime.llm_read_timeout_secs),
        max_retries: runtime.llm_max_retries,
        retry_base_delay: std::time::Duration::from_millis(runtime.llm_retry_base_delay_ms),
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfigBackedLlmProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<LlmEventSink>) -> Result<LlmOutput> {
        let provider = self.resolve_runtime_provider()?;
        provider.generate(request, sink).await
    }

    fn model_limits(&self) -> ModelLimits {
        match self.resolve_spec() {
            Ok(spec) => spec.limits,
            Err(error) => {
                log::error!("解析当前 LLM limits 失败: {error}");
                ModelLimits {
                    context_window: 128_000,
                    max_output_tokens: 8_192,
                }
            },
        }
    }

    fn supports_cache_metrics(&self) -> bool {
        self.resolve_runtime_provider()
            .map(|provider| provider.supports_cache_metrics())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
struct ResolvedLlmProviderSpec {
    cache_key: String,
    provider_id: String,
    api_kind: String,
    endpoint: String,
    api_key: String,
    model: String,
    limits: ModelLimits,
    client_config: LlmClientConfig,
    openai_capabilities: Option<OpenAiProviderCapabilities>,
}

fn resolve_model_limits(provider_kind: &str, model: &ModelConfig) -> ModelLimits {
    let default_context_window = match provider_kind {
        config_mode_helpers::PROVIDER_KIND_OPENAI => 128_000,
        _ => 128_000,
    };
    ModelLimits {
        context_window: model.context_limit.unwrap_or(default_context_window),
        max_output_tokens: model
            .max_tokens
            .map(|value| value as usize)
            .unwrap_or(8_192),
    }
}

fn resolve_openai_api_mode(profile: &astrcode_core::Profile) -> OpenAiApiMode {
    profile.api_mode.unwrap_or_else(|| {
        if profile
            .base_url
            .trim()
            .starts_with("https://api.openai.com")
        {
            OpenAiApiMode::Responses
        } else {
            OpenAiApiMode::ChatCompletions
        }
    })
}

fn resolve_openai_provider_capabilities(
    endpoint: &str,
    configured: Option<&OpenAiProfileCapabilities>,
) -> OpenAiProviderCapabilities {
    let mut resolved = OpenAiProviderCapabilities::for_endpoint(endpoint);
    if let Some(configured) = configured {
        if let Some(value) = configured.supports_prompt_cache_key {
            resolved.supports_prompt_cache_key = value;
        }
        if let Some(value) = configured.supports_stream_usage {
            resolved.supports_stream_usage = value;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use astrcode_adapter_storage::config_store::FileConfigStore;
    use astrcode_core::{Config, ModelConfig, Profile};
    use astrcode_plugin_host::{
        OPENAI_API_KIND, PluginDescriptor, ProviderContributionCatalog, ProviderDescriptor,
    };

    use super::ConfigBackedLlmProvider;
    use crate::{ConfigService, config_service_bridge::ServerConfigService};

    fn provider_with_config(
        config: Config,
        catalog: ProviderContributionCatalog,
        working_dir: &std::path::Path,
    ) -> ConfigBackedLlmProvider {
        let config_path = working_dir.join("config.json");
        let store = FileConfigStore::new(config_path);
        store.save(&config).expect("config should save");
        ConfigBackedLlmProvider::new(
            Arc::new(ServerConfigService::new(Arc::new(ConfigService::new(
                Arc::new(store),
            )))),
            working_dir.to_path_buf(),
            Arc::new(RwLock::new(catalog)),
        )
    }

    fn config_for_provider_kind(provider_kind: &str) -> Config {
        let mut model = ModelConfig::new("corp-model");
        model.max_tokens = Some(4096);
        model.context_limit = Some(128_000);
        Config {
            active_profile: "corp".to_string(),
            active_model: "corp-model".to_string(),
            profiles: vec![Profile {
                name: "corp".to_string(),
                provider_kind: provider_kind.to_string(),
                base_url: "https://api.example.test".to_string(),
                api_key: Some("literal:test-key".to_string()),
                models: vec![model],
                ..Profile::default()
            }],
            ..Config::default()
        }
    }

    #[test]
    fn resolve_spec_uses_registered_provider_id_before_api_kind() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let mut descriptor = PluginDescriptor::builtin("corp-provider", "Corp Provider");
        descriptor.providers.push(ProviderDescriptor {
            provider_id: "corp-openai".to_string(),
            api_kind: OPENAI_API_KIND.to_string(),
        });
        let catalog = ProviderContributionCatalog::from_descriptors(&[descriptor])
            .expect("catalog should build");
        let provider = provider_with_config(
            config_for_provider_kind("corp-openai"),
            catalog,
            temp.path(),
        );

        let spec = provider.resolve_spec().expect("provider should resolve");

        assert_eq!(spec.provider_id, "corp-openai");
        assert_eq!(spec.api_kind, OPENAI_API_KIND);
    }

    #[test]
    fn resolve_spec_keeps_api_kind_fallback_for_existing_openai_configs() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let mut descriptor = PluginDescriptor::builtin("renamed-openai", "Renamed OpenAI");
        descriptor.providers.push(ProviderDescriptor {
            provider_id: "renamed-openai".to_string(),
            api_kind: OPENAI_API_KIND.to_string(),
        });
        let catalog = ProviderContributionCatalog::from_descriptors(&[descriptor])
            .expect("catalog should build");
        let provider =
            provider_with_config(config_for_provider_kind("openai"), catalog, temp.path());

        let spec = provider.resolve_spec().expect("provider should resolve");

        assert_eq!(spec.provider_id, "renamed-openai");
        assert_eq!(spec.api_kind, OPENAI_API_KIND);
    }
}
