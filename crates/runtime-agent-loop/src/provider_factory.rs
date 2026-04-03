//! # Provider Factory（LLM 提供者工厂）
//!
//! ## 职责
//!
//! 负责根据工作目录解析配置并构建对应的 LLM Provider 实例（OpenAI/Anthropic）。
//! 是 AgentLoop 获取 LLM 能力的入口点。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：Turn 开始时，`turn_runner` 通过 `llm_cycle::build_provider()` 调用
//! - **输入**：工作目录路径（可选）
//! - **输出**：带有统一 `ModelLimits` 的 `Arc<dyn LlmProvider>`
//!
//! ## 依赖和协作
//!
//! - **使用** `astrcode_runtime_config` 加载并解析 `~/.astrcode/config.json`
//! - **使用** `resolve_model_for_profile` 选择活跃模型
//! - **使用** `astrcode_runtime_llm::AnthropicProvider/OpenAiProvider` 构建具体提供者
//! - ** Anthropic Provider 构造前会调用 Models API 拉取权威 limits，本地配置仅作兜底**
//! - 通过 `ProviderFactory` trait 抽象，支持热替换和/mock 测试
//!
//! ## 关键设计
//!
//! - `build_requires_blocking_pool()` 标记是否需要在线程池执行（磁盘 I/O 相关）
//! - 返回的 provider 已包含解析好的上下文窗口和 max_output_tokens，后续 agent loop 不再猜测

use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::Result;
use astrcode_runtime_llm::LlmProvider;

pub trait ProviderFactory: Send + Sync {
    /// 当提供程序构造执行阻塞 I/O 时返回 true，并且应在阻塞池上运行
    /// 而不是在 Tokio 工作线程上运行
    fn build_requires_blocking_pool(&self) -> bool {
        false
    }

    fn build_for_working_dir(&self, working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>>;
}

pub type DynProviderFactory = Arc<dyn ProviderFactory>;
