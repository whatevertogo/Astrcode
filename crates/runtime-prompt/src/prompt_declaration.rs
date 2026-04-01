use serde::{Deserialize, Serialize};

use crate::{BlockKind, RenderTarget};

/// Origin of a prompt declaration — kept separate from [`SkillSource`] because the two
/// may diverge (e.g. a future `PromptDeclarationSource::System` vs `SkillSource::User`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationSource {
    Builtin,
    #[default]
    Plugin,
    Mcp,
}

impl PromptDeclarationSource {
    pub fn as_tag(&self) -> &'static str {
        match self {
            Self::Builtin => "source:builtin",
            Self::Plugin => "source:plugin",
            Self::Mcp => "source:mcp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationKind {
    ToolGuide,
    #[default]
    ExtensionInstruction,
}

impl PromptDeclarationKind {
    pub fn as_block_kind(&self) -> BlockKind {
        match self {
            Self::ToolGuide => BlockKind::ToolGuide,
            Self::ExtensionInstruction => BlockKind::ExtensionInstruction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationRenderTarget {
    #[default]
    System,
    PrependUser,
    PrependAssistant,
    AppendUser,
    AppendAssistant,
}

impl PromptDeclarationRenderTarget {
    pub fn as_render_target(&self) -> RenderTarget {
        match self {
            Self::System => RenderTarget::System,
            Self::PrependUser => RenderTarget::PrependUser,
            Self::PrependAssistant => RenderTarget::PrependAssistant,
            Self::AppendUser => RenderTarget::AppendUser,
            Self::AppendAssistant => RenderTarget::AppendAssistant,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptDeclaration {
    pub block_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub render_target: PromptDeclarationRenderTarget,
    #[serde(default)]
    pub kind: PromptDeclarationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<i32>,
    #[serde(default)]
    pub always_include: bool,
    #[serde(default)]
    pub source: PromptDeclarationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}
