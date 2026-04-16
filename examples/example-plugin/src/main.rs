use std::{
    fs,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use astrcode_core::{
    AstrError, CancelToken, CapabilityKind, CapabilitySpec, InvocationMode, Result, SideEffect,
    Stability,
};
use astrcode_plugin::{CapabilityHandler, CapabilityRouter, EventEmitter, Worker};
use astrcode_protocol::plugin::{InvocationContext, PeerDescriptor, PeerRole};
use astrcode_sdk::{
    DeserializeOwned, PluginContext, Serialize, StreamWriter, ToolHandler, ToolRegistration,
    ToolResult,
};
use async_trait::async_trait;
use serde_json::{Value, json};

struct RegisteredToolAdapter {
    registration: ToolRegistration,
}

impl RegisteredToolAdapter {
    fn new<H, I, O>(handler: H) -> Self
    where
        H: ToolHandler<I, O> + 'static,
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
    {
        Self {
            registration: ToolRegistration::new(handler),
        }
    }
}

#[async_trait]
impl CapabilityHandler for RegisteredToolAdapter {
    fn capability_spec(&self) -> CapabilitySpec {
        self.registration.descriptor().clone()
    }

    async fn invoke(
        &self,
        input: Value,
        context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value> {
        let plugin_context = PluginContext::from(context);
        let stream = StreamWriter::default();
        let tool_name = self.registration.descriptor().name.to_string();
        if cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }
        let output = self
            .registration
            .handler()
            .execute_value(input, plugin_context, stream.clone())
            .await
            .map_err(|error| AstrError::ToolError {
                name: tool_name.clone(),
                reason: error.to_string(),
            })?;
        for chunk in stream.records().map_err(|error| AstrError::ToolError {
            name: tool_name.clone(),
            reason: error.to_string(),
        })? {
            events.delta(chunk.event, chunk.payload).await?;
        }
        if cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }
        Ok(output)
    }
}

#[derive(Default)]
struct WorkspaceSummaryTool;

impl ToolHandler for WorkspaceSummaryTool {
    fn descriptor(&self) -> CapabilitySpec {
        CapabilitySpec::builder("workspace.summary", CapabilityKind::tool())
            .description("Summarize the active coding workspace")
            .schema(
                json!({
                    "type": "object",
                    "properties": {}
                }),
                json!({
                    "type": "object",
                    "properties": {
                        "workspaceRoot": { "type": "string" },
                        "entries": { "type": "array" },
                        "markerFiles": { "type": "array" }
                    }
                }),
            )
            .invocation_mode(InvocationMode::Unary)
            .concurrency_safe(true)
            .profiles(["coding"])
            .tags(["example", "workspace"])
            .side_effect(SideEffect::None)
            .stability(Stability::Stable)
            .build()
            .expect("workspace summary capability spec should build")
    }

    fn execute(
        &self,
        _input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult<Value>> + Send>> {
        Box::pin(async move {
            let root = workspace_root(&context)?;
            stream.diagnostic("info", format!("Scanning workspace {}", root.display()))?;

            let mut entries = fs::read_dir(&root)?
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    let kind = entry
                        .file_type()
                        .ok()
                        .map(|kind| if kind.is_dir() { "dir" } else { "file" })
                        .unwrap_or("unknown");
                    json!({
                        "name": entry.file_name().to_string_lossy().into_owned(),
                        "kind": kind
                    })
                })
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| {
                left["name"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["name"].as_str().unwrap_or_default())
            });

            let marker_files = [
                "Cargo.toml",
                "package.json",
                "pnpm-workspace.yaml",
                "pyproject.toml",
                ".git",
            ]
            .into_iter()
            .filter(|candidate| root.join(candidate).exists())
            .collect::<Vec<_>>();

            Ok(json!({
                "workspaceRoot": root.to_string_lossy().into_owned(),
                "entryCount": entries.len(),
                "entries": entries,
                "markerFiles": marker_files
            }))
        })
    }
}

#[derive(Default)]
struct FilePreviewTool;

impl ToolHandler for FilePreviewTool {
    fn descriptor(&self) -> CapabilitySpec {
        CapabilitySpec::builder("file.preview", CapabilityKind::tool())
            .description("Read a short preview from a file inside the active workspace")
            .schema(
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {
                        "path": { "type": "string" },
                        "maxLines": { "type": "integer", "minimum": 1, "maximum": 200 }
                    }
                }),
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "preview": { "type": "string" },
                        "truncated": { "type": "boolean" }
                    }
                }),
            )
            .invocation_mode(InvocationMode::Unary)
            .concurrency_safe(true)
            .profiles(["coding"])
            .tags(["example", "file"])
            .side_effect(SideEffect::None)
            .stability(Stability::Stable)
            .build()
            .expect("file preview capability spec should build")
    }

    fn execute(
        self: &FilePreviewTool,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> Pin<Box<dyn std::future::Future<Output = ToolResult<Value>> + Send>> {
        Box::pin(async move {
            let root = workspace_root(&context)?;
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing required field 'path'".to_string())?;
            let max_lines = input
                .get("maxLines")
                .and_then(Value::as_u64)
                .unwrap_or(40)
                .min(200) as usize;

            let resolved = resolve_workspace_path(&root, path)?;
            stream.message_delta(format!("Previewing {}", resolved.display()))?;

            let content = fs::read_to_string(&resolved)?;
            let lines = content.lines().collect::<Vec<_>>();
            let truncated = lines.len() > max_lines;
            let preview = lines
                .into_iter()
                .take(max_lines)
                .collect::<Vec<_>>()
                .join("\n");

            Ok(json!({
                "path": resolved.strip_prefix(&root).unwrap_or(&resolved).to_string_lossy().into_owned(),
                "preview": preview,
                "truncated": truncated
            }))
        })
    }
}

fn workspace_root(context: &PluginContext) -> ToolResult<PathBuf> {
    if let Some(coding) = context.coding_profile() {
        if let Some(path) = coding.working_dir.or(coding.repo_root).map(PathBuf::from) {
            return Ok(path);
        }
    }

    if let Some(workspace) = &context.workspace {
        if let Some(path) = workspace
            .working_dir
            .as_ref()
            .or(workspace.repo_root.as_ref())
            .map(PathBuf::from)
        {
            return Ok(path);
        }
    }

    Err("workspace path is missing from coding context".into())
}

fn resolve_workspace_path(root: &Path, candidate: &str) -> ToolResult<PathBuf> {
    let joined = if Path::new(candidate).is_absolute() {
        PathBuf::from(candidate)
    } else {
        root.join(candidate)
    };

    let canonical_root = root.canonicalize()?;
    let canonical_path = joined.canonicalize()?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("requested file is outside the active workspace".into());
    }
    Ok(canonical_path)
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut router = CapabilityRouter::default();
    router.register_arc(Arc::new(RegisteredToolAdapter::new(WorkspaceSummaryTool)))?;
    router.register_arc(Arc::new(RegisteredToolAdapter::new(FilePreviewTool)))?;

    let worker = Worker::from_stdio(
        PeerDescriptor {
            id: "astrcode-example-plugin".to_string(),
            name: "astrcode-example-plugin".to_string(),
            role: PeerRole::Worker,
            version: env!("CARGO_PKG_VERSION").to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: json!({ "example": true }),
        },
        router,
        None,
    )?;
    worker.run().await
}
