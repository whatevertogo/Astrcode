use std::sync::Arc;

use astrcode_core::{
    LlmEventSink, LlmOutput, LlmProvider, LlmRequest, PromptBuildOutput, PromptBuildRequest,
    PromptProvider, ResourceProvider, ResourceReadResult, ResourceRequestContext, ToolCallRequest,
    ToolContext, ToolExecutionResult,
};

use crate::{error::KernelError, registry::CapabilityRouter};

#[derive(Clone)]
pub struct KernelGateway {
    llm: Arc<dyn LlmProvider>,
    prompt: Arc<dyn PromptProvider>,
    resource: Arc<dyn ResourceProvider>,
    capabilities: CapabilityRouter,
}

impl KernelGateway {
    pub fn new(
        capabilities: CapabilityRouter,
        llm: Arc<dyn LlmProvider>,
        prompt: Arc<dyn PromptProvider>,
        resource: Arc<dyn ResourceProvider>,
    ) -> Self {
        Self {
            llm,
            prompt,
            resource,
            capabilities,
        }
    }

    pub fn capabilities(&self) -> &CapabilityRouter {
        &self.capabilities
    }

    pub fn model_limits(&self) -> astrcode_core::ModelLimits {
        self.llm.model_limits()
    }

    pub async fn invoke_tool(
        &self,
        call: &ToolCallRequest,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        self.capabilities.execute_tool(call, ctx).await
    }

    pub async fn call_llm(
        &self,
        request: LlmRequest,
        sink: Option<LlmEventSink>,
    ) -> Result<LlmOutput, KernelError> {
        self.llm
            .generate(request, sink)
            .await
            .map_err(KernelError::from)
    }

    pub async fn build_prompt(
        &self,
        request: PromptBuildRequest,
    ) -> Result<PromptBuildOutput, KernelError> {
        self.prompt
            .build_prompt(request)
            .await
            .map_err(KernelError::from)
    }

    pub async fn read_resource(
        &self,
        uri: &str,
        context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult, KernelError> {
        self.resource
            .read_resource(uri, context)
            .await
            .map_err(KernelError::from)
    }
}
