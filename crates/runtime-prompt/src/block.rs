use std::borrow::Cow;
use std::collections::HashMap;

use super::template::PromptTemplate;

/// Semantic classification of a prompt block.
///
/// Determines default priority ordering in the assembled system prompt.
/// Lower priority values appear earlier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Agent identity (who the AI is). Priority 100.
    Identity,
    // 预留给未来的系统 prompt 块——当 PromptComposer 需要将多个 identity/rule 片段
    // 合并为一个统一的 system prompt 时，此变体将作为合成结果的 BlockKind。
    #[allow(dead_code)]
    SystemPrompt,
    /// Working directory, OS, date, tool list. Priority 300.
    Environment,
    /// User-level rules from ~/.astrcode/AGENTS.md. Priority 400.
    UserRules,
    /// Project-level rules from ./AGENTS.md. Priority 500.
    ProjectRules,
    /// Per-tool usage guides (summary + detailed guide). Priority 550.
    ToolGuide,
    /// Multi-tool workflow guides (e.g. "read before edit"). Priority 560.
    SkillGuide,
    /// Plugin/MCP-injected prompt instructions. Priority 580.
    ExtensionInstruction,
    /// Legacy skill summary block (tool name list). Priority 600.
    Skill,
    /// Few-shot example message pairs. Priority 700.
    FewShotExamples,
}

impl BlockKind {
    pub fn default_priority(self) -> i32 {
        match self {
            Self::Identity => 100,
            Self::SystemPrompt => 200,
            Self::Environment => 300,
            Self::UserRules => 400,
            Self::ProjectRules => 500,
            Self::ToolGuide => 550,
            Self::SkillGuide => 560,
            Self::ExtensionInstruction => 580,
            Self::Skill => 600,
            Self::FewShotExamples => 700,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    System,
    PrependUser,
    PrependAssistant,
    AppendUser,
    AppendAssistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationPolicy {
    Inherit,
    Skip,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BlockCondition {
    #[default]
    Always,
    StepEquals(usize),
    FirstStepOnly,
    HasTool(String),
    VarEquals {
        key: String,
        expected: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockMetadata {
    pub tags: Vec<Cow<'static, str>>,
    pub category: Option<Cow<'static, str>>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockContent {
    Text(String),
    Template(PromptTemplate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSpec {
    pub id: Cow<'static, str>,
    pub kind: BlockKind,
    pub title: Cow<'static, str>,
    pub content: BlockContent,
    pub priority: Option<i32>,
    pub condition: BlockCondition,
    pub dependencies: Vec<Cow<'static, str>>,
    pub validation_policy: ValidationPolicy,
    pub render_target: RenderTarget,
    pub metadata: BlockMetadata,
    pub vars: HashMap<String, String>,
}

impl BlockSpec {
    pub fn system_text(
        id: impl Into<Cow<'static, str>>,
        kind: BlockKind,
        title: impl Into<Cow<'static, str>>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            content: BlockContent::Text(content.into()),
            priority: None,
            condition: BlockCondition::Always,
            dependencies: Vec::new(),
            validation_policy: ValidationPolicy::Inherit,
            render_target: RenderTarget::System,
            metadata: BlockMetadata::default(),
            vars: HashMap::new(),
        }
    }

    pub fn system_template(
        id: impl Into<Cow<'static, str>>,
        kind: BlockKind,
        title: impl Into<Cow<'static, str>>,
        template: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            content: BlockContent::Template(PromptTemplate::new(template)),
            ..Self::system_text(id, kind, title, String::new())
        }
    }

    pub fn message_text(
        id: impl Into<Cow<'static, str>>,
        kind: BlockKind,
        title: impl Into<Cow<'static, str>>,
        content: impl Into<String>,
        render_target: RenderTarget,
    ) -> Self {
        Self {
            render_target,
            ..Self::system_text(id, kind, title, content)
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn with_condition(mut self, condition: BlockCondition) -> Self {
        self.condition = condition;
        self
    }

    pub fn depends_on(mut self, dependency: impl Into<Cow<'static, str>>) -> Self {
        self.dependencies.push(dependency.into());
        self
    }

    pub fn with_tag(mut self, tag: impl Into<Cow<'static, str>>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<Cow<'static, str>>) -> Self {
        self.metadata.category = Some(category.into());
        self
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.metadata.origin = Some(origin.into());
        self
    }

    // 仅在测试中使用；生产代码通过 contributor 设置 vars
    #[allow(dead_code)]
    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    pub fn effective_priority(&self) -> i32 {
        self.priority
            .unwrap_or_else(|| self.kind.default_priority())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBlock {
    pub id: String,
    pub kind: BlockKind,
    pub title: String,
    pub content: String,
    pub priority: i32,
    pub metadata: BlockMetadata,
    pub insertion_order: usize,
}

impl PromptBlock {
    pub fn new(
        id: impl Into<String>,
        kind: BlockKind,
        title: impl Into<String>,
        content: impl Into<String>,
        priority: i32,
        metadata: BlockMetadata,
        insertion_order: usize,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            content: content.into(),
            priority,
            metadata,
            insertion_order,
        }
    }
}
