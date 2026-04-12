//! 桥接 `adapter-prompt` 的 `PromptComposer` 与 `core::ports::PromptProvider`。
//!
//! `core::ports::PromptProvider` 是 kernel 消费的简化端口接口，
//! 本模块将其适配到 `PromptComposer` 的完整 prompt 构建能力上。

use astrcode_core::{
    Result, SystemPromptBlock, SystemPromptLayer,
    ports::{PromptBuildOutput, PromptBuildRequest, PromptProvider},
};
use async_trait::async_trait;

use crate::{composer::PromptComposer, context::PromptContext};

/// adapter-prompt 的 PromptLayer → core 的 SystemPromptLayer
fn convert_layer(layer: crate::PromptLayer) -> SystemPromptLayer {
    match layer {
        crate::PromptLayer::Stable => SystemPromptLayer::Stable,
        crate::PromptLayer::SemiStable => SystemPromptLayer::SemiStable,
        crate::PromptLayer::Inherited => SystemPromptLayer::Inherited,
        crate::PromptLayer::Dynamic => SystemPromptLayer::Dynamic,
        crate::PromptLayer::Unspecified => SystemPromptLayer::Unspecified,
    }
}

/// 基于 `PromptComposer` 的 `PromptProvider` 实现。
///
/// 将 `core::ports::PromptBuildRequest` 转为 `PromptContext`，
/// 调用 `PromptComposer::build()` 后将 `PromptPlan` 渲染为 system prompt。
pub struct ComposerPromptProvider {
    composer: PromptComposer,
}

impl ComposerPromptProvider {
    pub fn new(composer: PromptComposer) -> Self {
        Self { composer }
    }

    /// 使用默认贡献者创建。
    pub fn with_defaults() -> Self {
        Self {
            composer: PromptComposer::with_defaults(),
        }
    }
}

impl std::fmt::Debug for ComposerPromptProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposerPromptProvider").finish()
    }
}

#[async_trait]
impl PromptProvider for ComposerPromptProvider {
    async fn build_prompt(&self, request: PromptBuildRequest) -> Result<PromptBuildOutput> {
        let ctx = PromptContext {
            working_dir: request.working_dir.to_string_lossy().to_string(),
            capability_specs: request.capabilities,
            step_index: 0,
            turn_index: 0,
            ..Default::default()
        };

        let output = self
            .composer
            .build(&ctx)
            .await
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;

        let system_prompt = output.plan.render_system().unwrap_or_default();

        // 将 ordered system blocks 转为 core 的 SystemPromptBlock 格式
        let system_prompt_blocks: Vec<SystemPromptBlock> = output
            .plan
            .ordered_system_blocks()
            .into_iter()
            .map(|block| SystemPromptBlock {
                title: block.title.clone(),
                content: block.content.clone(),
                cache_boundary: false,
                layer: convert_layer(block.layer),
            })
            .collect();

        Ok(PromptBuildOutput {
            system_prompt,
            system_prompt_blocks,
            metadata: serde_json::json!({
                "extra_tools_count": output.plan.extra_tools.len(),
                "diagnostics_count": output.diagnostics.items.len(),
            }),
        })
    }
}
