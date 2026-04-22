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
use astrcode_adapter_mcp::{core_port::McpResourceProvider, manager::McpConnectionManager};
use astrcode_adapter_prompt::{
    core_port::ComposerPromptProvider, layered_builder::default_layered_prompt_builder,
};
use astrcode_adapter_storage::config_store::FileConfigStore;
use astrcode_application::{
    ApplicationError, ProfileResolutionService,
    config::{
        ConfigService, PROVIDER_KIND_OPENAI, api_key, resolve_current_model,
        resolve_openai_chat_completions_api_url, resolve_openai_responses_api_url,
    },
    execution::ProfileProvider,
};
use astrcode_core::config::{OpenAiApiMode, OpenAiProfileCapabilities};

use super::deps::core::{
    AgentProfile, AstrError, LlmEventSink, LlmOutput, LlmProvider, LlmRequest, ModelConfig,
    PromptProvider, ResolvedRuntimeConfig, ResourceProvider, Result, resolve_runtime_config,
};

pub(crate) fn build_llm_provider(
    config_service: Arc<ConfigService>,
    working_dir: PathBuf,
) -> Arc<dyn LlmProvider> {
    Arc::new(ConfigBackedLlmProvider::new(config_service, working_dir))
}

pub(crate) fn build_prompt_provider() -> Arc<dyn PromptProvider> {
    Arc::new(ComposerPromptProvider::new(default_layered_prompt_builder()))
}

pub(crate) fn build_resource_provider(
    manager: Arc<McpConnectionManager>,
) -> Arc<dyn ResourceProvider> {
    Arc::new(McpResourceProvider::new(manager))
}

pub(crate) fn build_config_service(config_path: PathBuf) -> Result<Arc<ConfigService>> {
    let config_store = FileConfigStore::new(config_path);
    Ok(Arc::new(ConfigService::new(Arc::new(config_store))))
}

pub(crate) fn build_profile_resolution_service(
    loader: AgentProfileLoader,
) -> Result<Arc<ProfileResolutionService>> {
    let provider: Arc<dyn ProfileProvider> = Arc::new(LoaderBackedProfileProvider { loader });
    Ok(Arc::new(ProfileResolutionService::new(provider)))
}

struct ConfigBackedLlmProvider {
    config_service: Arc<ConfigService>,
    working_dir: PathBuf,
    providers: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
}

struct LoaderBackedProfileProvider {
    loader: AgentProfileLoader,
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
    fn new(config_service: Arc<ConfigService>, working_dir: PathBuf) -> Self {
        Self {
            config_service,
            working_dir,
            providers: RwLock::new(HashMap::new()),
        }
    }

    fn resolve_spec(&self) -> std::result::Result<ResolvedLlmProviderSpec, ApplicationError> {
        let config = self
            .config_service
            .load_overlayed_config(Some(self.working_dir.as_path()))?;
        let selection = resolve_current_model(&config)?;
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
        let api_key = api_key::resolve_api_key(profile)
            .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        let limits = resolve_model_limits(&profile.provider_kind, model);
        let runtime = resolve_runtime_config(&config.runtime);
        let client_config = client_config_from_runtime(&runtime);
        if profile.provider_kind != PROVIDER_KIND_OPENAI {
            return Err(ApplicationError::InvalidArgument(format!(
                "unsupported provider_kind '{}'",
                profile.provider_kind
            )));
        }
        let api_mode = resolve_openai_api_mode(profile);
        let endpoint = match api_mode {
            OpenAiApiMode::ChatCompletions => {
                resolve_openai_chat_completions_api_url(&profile.base_url)
            },
            OpenAiApiMode::Responses => resolve_openai_responses_api_url(&profile.base_url),
        };
        let openai_capabilities = Some(resolve_openai_provider_capabilities(
            endpoint.as_str(),
            profile.openai_capabilities.as_ref(),
        ));

        Ok(ResolvedLlmProviderSpec {
            cache_key: format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                profile.provider_kind,
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
            provider_kind: profile.provider_kind.clone(),
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

        if spec.provider_kind != PROVIDER_KIND_OPENAI {
            return Err(AstrError::Validation(format!(
                "unsupported provider_kind '{}'",
                spec.provider_kind
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
    provider_kind: String,
    endpoint: String,
    api_key: String,
    model: String,
    limits: ModelLimits,
    client_config: LlmClientConfig,
    openai_capabilities: Option<OpenAiProviderCapabilities>,
}

fn resolve_model_limits(provider_kind: &str, model: &ModelConfig) -> ModelLimits {
    let default_context_window = match provider_kind {
        PROVIDER_KIND_OPENAI => 128_000,
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
