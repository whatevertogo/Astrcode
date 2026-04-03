//! Prompt 组装与 Skill 加载库。
//!
//! # 架构概述
//!
//! 本 crate 实现了 agent 循环中的 prompt 组装管线，采用**贡献者模式**（contributor pattern）：
//! 每个 [`PromptContributor`] 负责生成一段 prompt 内容（称为 [`BlockSpec`]），
//! [`PromptComposer`] 收集所有贡献、解析依赖、渲染模板，最终产出 [`PromptPlan`]。
//!
//! # 两阶段 Skill 模型
//!
//! Skill 系统遵循两阶段加载策略：
//! 1. **索引阶段**：system prompt 中仅暴露 skill 的名称和描述（由 `SkillSummaryContributor` 负责）
//! 2. **按需加载**：当模型调用内置 `Skill` tool 时，才加载完整的 skill 正文和资产
//!
//! # 核心概念
//!
//! - **Block**：prompt 的最小组成单元，带有语义分类（[`BlockKind`]）、优先级、条件、依赖等元数据
//! - **Contributor**：独立的 prompt 内容提供者，如身份、环境、规则、工具指南等
//! - **Composer**：管线编排器，负责收集、去重、拓扑排序、渲染和验证
//! - **Skill**：可插拔的专业能力模块，通过 `SKILL.md` 定义，具体来源由 `SkillCatalog` 统一解析
//!
//! # 设计原则
//!
//! - 每个 contributor 保持编译隔离，通过 trait 接口组合
//! - prompt 块支持条件渲染（如仅首步、特定工具可用时）
//! - 依赖解析采用波前式拓扑排序，自动检测循环依赖
//! - 内置 skill 资源由 `build.rs` 在编译期打包，避免手写 `include_str!` 清单

pub mod block;
mod builtin_skills;
pub mod composer;
pub mod context;
pub mod contribution;
pub mod contributor;
pub mod contributors;
pub mod diagnostics;
pub mod plan;
pub mod prompt_declaration;
pub mod skill_catalog;
pub mod skill_loader;
pub mod skill_spec;
pub mod template;

pub use block::{
    BlockCondition, BlockContent, BlockKind, BlockSpec, PromptBlock, RenderTarget, ValidationPolicy,
};
pub use builtin_skills::load_builtin_skills;
pub use composer::{PromptComposer, PromptComposerOptions, ValidationLevel};
pub use context::PromptContext;
pub use contribution::{append_unique_tools, PromptContribution};
pub use contributor::PromptContributor;
pub use diagnostics::{DiagnosticLevel, PromptDiagnostics};
pub use plan::PromptPlan;
pub use prompt_declaration::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource,
};
pub use skill_catalog::{merge_skill_layers, SkillCatalog};
pub use skill_loader::{
    collect_asset_files, load_project_skills, load_user_skills, parse_skill_md,
    skill_roots_cache_marker, SkillFrontmatter, SKILL_FILE_NAME, SKILL_TOOL_NAME,
};
pub use skill_spec::{is_valid_skill_name, normalize_skill_name, SkillSource, SkillSpec};
pub use template::TemplateRenderError;
