use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticReason {
    ConditionSkipped { condition: String },
    MissingDependency { dependency_id: String },
    TemplateVariableMissing { variable: String },
    RenderFailed { message: String },
    ValidationFailed { message: String },
    ContributorCacheHit,
    ContributorCacheMiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDiagnostic {
    pub level: DiagnosticLevel,
    pub block_id: Option<String>,
    pub contributor_id: Option<String>,
    pub reason: DiagnosticReason,
    pub suggestion: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptDiagnostics {
    pub items: Vec<PromptDiagnostic>,
}

impl PromptDiagnostics {
    pub fn push(&mut self, diagnostic: PromptDiagnostic) {
        self.items.push(diagnostic);
    }
}
