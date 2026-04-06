//! 插件集成测试。
//!
//! 覆盖：
//! - 内置工具与插件工具统一路由执行

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    CancelToken, Phase, PluginManifest, PluginType, StorageEvent, ToolCallRequest,
    UserMessageOrigin,
};
use astrcode_plugin::Supervisor;
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use astrcode_runtime_llm::LlmOutput;
use serde_json::json;

use super::fixtures::*;
use crate::AgentLoop;

#[tokio::test]
async fn unified_capability_router_executes_builtin_and_plugin_tools() {
    let workspace = tempfile::tempdir().expect("tempdir should be created");
    fs::write(
        workspace.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("Cargo.toml should be written");
    fs::create_dir_all(workspace.path().join("src")).expect("src dir should be created");
    fs::write(
        workspace.path().join("src").join("lib.rs"),
        "pub fn demo() {}\n",
    )
    .expect("lib.rs should be written");

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-builtin".to_string(),
                        name: "quickTool".to_string(),
                        args: json!({}),
                    },
                    ToolCallRequest {
                        id: "call-plugin".to_string(),
                        name: "workspace.summary".to_string(),
                        args: json!({}),
                    },
                ],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
                usage: None,
                finish_reason: Default::default(),
            },
        ])),
        delay: std::time::Duration::from_millis(0),
    });

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime crate should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf();
    let manifest = PluginManifest {
        name: "repo-inspector".to_string(),
        version: "0.1.0".to_string(),
        description: "example plugin".to_string(),
        plugin_type: vec![PluginType::Tool],
        capabilities: vec![],
        executable: Some("cargo".to_string()),
        args: vec![
            "run".to_string(),
            "-p".to_string(),
            "astrcode-example-plugin".to_string(),
            "--quiet".to_string(),
        ],
        working_dir: Some(repo_root.to_string_lossy().into_owned()),
        repository: None,
    };
    let supervisor = Supervisor::start(
        &manifest,
        PeerDescriptor {
            id: "runtime-test-supervisor".to_string(),
            name: "runtime-test-supervisor".to_string(),
            role: PeerRole::Supervisor,
            version: env!("CARGO_PKG_VERSION").to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: serde_json::Value::Null,
        },
    )
    .await
    .expect("supervisor should start");

    let mut capability_builder = astrcode_runtime_registry::CapabilityRouter::builder();
    for invoker in astrcode_runtime_registry::ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build()
        .into_capability_invokers()
        .expect("tool descriptors should build")
    {
        capability_builder = capability_builder.register_invoker(invoker);
    }
    for invoker in supervisor.capability_invokers() {
        capability_builder = capability_builder.register_invoker(invoker);
    }
    let capabilities = capability_builder
        .build()
        .expect("capability router should build");

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities);
    let state = astrcode_core::AgentState {
        session_id: "test".into(),
        working_dir: workspace.path().to_path_buf(),
        messages: vec![astrcode_core::LlmMessage::User {
            content: "summarize workspace".into(),
            origin: UserMessageOrigin::User,
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let (events, mut on_event) = collect_events();

    loop_runner
        .run_turn(&state, "turn-plugin", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");

    // 在关闭 supervisor 前先把断言结果提取出来，避免同步锁跨 await 持有。
    let (saw_quick_tool, saw_workspace_summary) = {
        let events = events.lock().expect("lock");
        let saw_quick_tool = events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_name,
                    output,
                    ..
                } if tool_name == "quickTool" && output == "ok"
            )
        });
        let saw_workspace_summary = events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_name,
                    output,
                    ..
                } if tool_name == "workspace.summary"
                    && output.contains("Cargo.toml")
                    && output.contains("\"workspaceRoot\"")
            )
        });
        (saw_quick_tool, saw_workspace_summary)
    };
    assert!(saw_quick_tool);
    assert!(saw_workspace_summary);

    supervisor
        .shutdown()
        .await
        .expect("supervisor should shut down");
}
