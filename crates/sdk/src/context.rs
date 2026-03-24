use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSelection {
    pub start_line: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_column: Option<u64>,
    pub end_line: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingProfileContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub open_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TextSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
    #[serde(default)]
    pub extras: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginContext {
    pub request_id: String,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub workspace: Option<WorkspaceContext>,
    pub profile: String,
    pub profile_context: Value,
}

impl Default for PluginContext {
    fn default() -> Self {
        Self {
            request_id: String::new(),
            session_id: None,
            trace_id: None,
            workspace: None,
            profile: "coding".to_string(),
            profile_context: Value::Null,
        }
    }
}

impl PluginContext {
    pub fn coding_profile(&self) -> Option<CodingProfileContext> {
        if self.profile != "coding" {
            return None;
        }
        serde_json::from_value(self.profile_context.clone()).ok()
    }
}

impl From<astrcode_protocol::plugin::InvocationContext> for PluginContext {
    fn from(value: astrcode_protocol::plugin::InvocationContext) -> Self {
        Self {
            request_id: value.request_id,
            session_id: value.session_id,
            trace_id: value.trace_id,
            workspace: value.workspace.map(|workspace| WorkspaceContext {
                working_dir: workspace.working_dir,
                repo_root: workspace.repo_root,
                branch: workspace.branch,
            }),
            profile: value.profile,
            profile_context: value.profile_context,
        }
    }
}
