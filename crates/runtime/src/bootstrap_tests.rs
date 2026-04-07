//! # 引导测试 (Bootstrap Tests)
//!
//! 验证运行时引导流程的正确性，包括：
//! - 插件初始化成功时的完整引导
//! - 插件初始化失败时的容错引导
//! - 能力冲突时的确定性解决（先到先得）
//! - 托管组件的有序关闭
//!
//! ## 设计
//!
//! 使用 `FakeInitializer` 模拟插件初始化过程，
//! 通过预定义的响应映射控制每个插件的初始化结果。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use astrcode_core::{
    CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult, CapabilityInvoker,
    CapabilityKind, HookEvent, HookHandler, HookInput, HookOutcome, ManagedRuntimeComponent,
    PluginHealth, PluginState, PluginType, Result, SideEffectLevel, StabilityLevel,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    ComposerOptionKind, ComposerOptionsRequest,
    bootstrap::{RuntimeBootstrap, bootstrap_runtime_from_manifests},
    runtime_surface_assembler::{
        LoadedPlugin, ManagedPluginComponent, ManagedPluginHealth, PluginInitializer,
        conflicting_capability_name,
    },
    test_support::TestEnvGuard,
};

#[derive(Clone)]
struct FakeInitializer {
    responses: HashMap<String, FakePluginResponse>,
}

#[derive(Clone)]
enum FakePluginResponse {
    Loaded(LoadedPlugin),
    Failed(String),
}

#[async_trait]
impl PluginInitializer for FakeInitializer {
    async fn initialize(
        &self,
        manifest: &astrcode_core::PluginManifest,
    ) -> std::result::Result<LoadedPlugin, astrcode_core::AstrError> {
        match self
            .responses
            .get(&manifest.name)
            .expect("initializer response should exist")
        {
            FakePluginResponse::Loaded(loaded) => Ok(LoadedPlugin {
                component: loaded.component.clone(),
                capabilities: loaded.capabilities.clone(),
                declared_skills: loaded.declared_skills.clone(),
                contribution: crate::runtime_surface_assembler::RuntimeSurfaceContribution {
                    capability_invokers: loaded.contribution.capability_invokers.clone(),
                    prompt_declarations: loaded.contribution.prompt_declarations.clone(),
                    skills: loaded.contribution.skills.clone(),
                    hook_handlers: loaded.contribution.hook_handlers.clone(),
                },
            }),
            FakePluginResponse::Failed(message) => {
                Err(astrcode_core::AstrError::Internal(message.clone()))
            },
        }
    }
}

struct FakeManagedComponent {
    name: String,
    shutdowns: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ManagedRuntimeComponent for FakeManagedComponent {
    fn component_name(&self) -> String {
        self.name.clone()
    }

    async fn shutdown_component(&self) -> Result<()> {
        self.shutdowns
            .lock()
            .expect("shutdown log lock")
            .push(self.name.clone());
        Ok(())
    }
}

#[async_trait]
impl ManagedPluginComponent for FakeManagedComponent {
    async fn health_report(
        &self,
    ) -> std::result::Result<ManagedPluginHealth, astrcode_core::AstrError> {
        Ok(ManagedPluginHealth {
            health: PluginHealth::Healthy,
            message: None,
        })
    }
}

struct FakeCapabilityInvoker {
    descriptor: CapabilityDescriptor,
}

#[async_trait]
impl CapabilityInvoker for FakeCapabilityInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.descriptor.clone()
    }

    async fn invoke(
        &self,
        _payload: Value,
        _ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        Ok(CapabilityExecutionResult::ok(
            self.descriptor.name.clone(),
            Value::Null,
        ))
    }
}

fn manifest(name: &str) -> astrcode_core::PluginManifest {
    astrcode_core::PluginManifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        description: format!("{name} plugin"),
        plugin_type: vec![PluginType::Tool],
        capabilities: Vec::new(),
        executable: Some("plugin.exe".to_string()),
        args: Vec::new(),
        working_dir: None,
        repository: None,
    }
}

fn capability(name: &str) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind: CapabilityKind::tool(),
        description: format!("{name} capability"),
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        streaming: false,
        concurrency_safe: false,
        compact_clearable: false,
        profiles: vec!["coding".to_string()],
        tags: Vec::new(),
        permissions: Vec::new(),
        side_effect: SideEffectLevel::None,
        stability: StabilityLevel::Stable,
        metadata: Value::Null,
    }
}

fn loaded_plugin(
    plugin_name: &str,
    capability_names: &[&str],
    shutdowns: Arc<Mutex<Vec<String>>>,
) -> LoadedPlugin {
    let capabilities = capability_names
        .iter()
        .map(|name| capability(name))
        .collect::<Vec<_>>();
    let invokers = capabilities
        .iter()
        .cloned()
        .map(|descriptor| {
            Arc::new(FakeCapabilityInvoker { descriptor }) as Arc<dyn CapabilityInvoker>
        })
        .collect();

    LoadedPlugin {
        component: Arc::new(FakeManagedComponent {
            name: plugin_name.to_string(),
            shutdowns,
        }),
        capabilities,
        declared_skills: Vec::new(),
        contribution: crate::runtime_surface_assembler::RuntimeSurfaceContribution {
            capability_invokers: invokers,
            prompt_declarations: Vec::new(),
            skills: Vec::new(),
            hook_handlers: Vec::new(),
        },
    }
}

struct BlockingPreCompactHook;

#[async_trait]
impl HookHandler for BlockingPreCompactHook {
    fn name(&self) -> &str {
        "bootstrap-blocking-pre-compact-hook"
    }

    fn event(&self) -> HookEvent {
        HookEvent::PreCompact
    }

    fn matches(&self, _input: &HookInput) -> bool {
        true
    }

    async fn run(&self, _input: &HookInput) -> Result<HookOutcome> {
        Ok(HookOutcome::Block {
            reason: "blocked by plugin hook".to_string(),
        })
    }
}

fn declared_skill(name: &str) -> astrcode_protocol::plugin::SkillDescriptor {
    astrcode_protocol::plugin::SkillDescriptor {
        name: name.to_string(),
        description: format!("Use {name}"),
        guide: format!("# {name}\nUse it."),
        allowed_tools: vec!["shell".to_string()],
        assets: vec![],
        metadata: json!({}),
    }
}

fn bootstrap_from(
    manifests: Vec<astrcode_core::PluginManifest>,
    initializer: FakeInitializer,
) -> (RuntimeBootstrap, TestEnvGuard) {
    let guard = TestEnvGuard::new();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let bootstrap = runtime
        .block_on(async { bootstrap_runtime_from_manifests(manifests, &initializer).await })
        .expect("runtime bootstrap should succeed");
    runtime.block_on(async {
        bootstrap.plugin_load_handle.wait_completed().await;
    });
    (bootstrap, guard)
}

fn current_dir_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct CurrentDirGuard {
    _lock: MutexGuard<'static, ()>,
    previous_dir: PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &std::path::Path) -> Self {
        let lock = match current_dir_lock().lock() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        let previous_dir = std::env::current_dir().expect("current dir should resolve");
        std::env::set_current_dir(path).expect("current dir should change");
        Self {
            _lock: lock,
            previous_dir,
        }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous_dir).expect("current dir should restore");
    }
}

#[test]
fn bootstrap_without_plugins_keeps_builtin_capabilities() {
    let initializer = FakeInitializer {
        responses: Default::default(),
    };
    let (bootstrap, _guard) = bootstrap_from(Vec::new(), initializer);

    assert!(
        bootstrap
            .coordinator
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.name == "shell")
    );
    assert!(
        bootstrap
            .coordinator
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.name == "Skill")
    );
    assert!(
        bootstrap
            .coordinator
            .plugin_registry()
            .snapshot()
            .is_empty()
    );
    assert!(
        bootstrap
            .agent_profiles
            .read()
            .expect("agent profile registry lock")
            .get("explore")
            .is_some()
    );
}

#[test]
fn bootstrap_loads_agent_profiles_from_user_level_agents_dir() {
    let guard = TestEnvGuard::new();
    let agents_dir = guard.home_dir().join(".astrcode").join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir should be created");
    std::fs::write(
        agents_dir.join("explore.md"),
        r#"---
name: explore
description: 用户级代码探索器
tools: ["readFile"]
---
优先阅读目录结构，再读取关键文件。
"#,
    )
    .expect("agent definition should be written");

    let initializer = FakeInitializer {
        responses: Default::default(),
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let bootstrap = runtime
        .block_on(async { bootstrap_runtime_from_manifests(Vec::new(), &initializer).await })
        .expect("runtime bootstrap should succeed");

    let agent_profiles = bootstrap
        .agent_profiles
        .read()
        .expect("agent profile registry lock");
    let explore = agent_profiles
        .get("explore")
        .expect("explore profile should exist");
    assert_eq!(explore.description, "用户级代码探索器");
    assert_eq!(explore.allowed_tools, vec!["readFile".to_string()]);

    let service_explore = bootstrap
        .service
        .agent_profiles()
        .get("explore")
        .expect("service should expose same profile registry")
        .description
        .clone();
    assert_eq!(service_explore, "用户级代码探索器");
}

#[test]
fn bootstrap_does_not_resolve_project_agents_from_process_cwd() {
    let guard = TestEnvGuard::new();
    let workspace = tempfile::tempdir().expect("tempdir should be created");
    let agent_dir = workspace.path().join(".astrcode").join("agents");
    std::fs::create_dir_all(&agent_dir).expect("agent dir should be created");
    std::fs::write(
        agent_dir.join("cwd-only.md"),
        r#"---
name: cwd-only
description: cwd scoped agent
tools: ["readFile"]
---
Only visible when working dir is explicit.
"#,
    )
    .expect("agent definition should be written");
    let _cwd = CurrentDirGuard::enter(workspace.path());

    let initializer = FakeInitializer {
        responses: Default::default(),
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let bootstrap = runtime
        .block_on(async { bootstrap_runtime_from_manifests(Vec::new(), &initializer).await })
        .expect("runtime bootstrap should succeed");

    assert!(
        bootstrap.service.agent_profiles().get("cwd-only").is_none(),
        "bootstrap must not silently bind agent resolution to process cwd"
    );
    drop(guard);
}

#[tokio::test]
async fn bootstrap_fails_fast_for_invalid_agent_markdown() {
    let guard = TestEnvGuard::new();
    let agents_dir = guard.home_dir().join(".astrcode").join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir should be created");
    std::fs::write(
        agents_dir.join("broken.md"),
        r#"---
name: broken
description: ""
---
Broken agent.
"#,
    )
    .expect("invalid agent definition should be written");

    let initializer = FakeInitializer {
        responses: Default::default(),
    };
    let error = match bootstrap_runtime_from_manifests(Vec::new(), &initializer).await {
        Ok(_) => panic!("invalid agent definition should fail bootstrap"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("invalid agent frontmatter"),
        "unexpected bootstrap error: {error}"
    );
}

#[test]
fn bootstrap_records_initialized_and_failed_plugins_without_aborting_server() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initializer = FakeInitializer {
        responses: HashMap::from([
            (
                "alpha".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "alpha",
                    &["tool.alpha"],
                    Arc::clone(&shutdowns),
                )),
            ),
            (
                "beta".to_string(),
                FakePluginResponse::Failed("handshake failed".to_string()),
            ),
        ]),
    };

    let (bootstrap, _guard) =
        bootstrap_from(vec![manifest("alpha"), manifest("beta")], initializer);
    let registry = bootstrap.coordinator.plugin_registry();
    let alpha = registry.get("alpha").expect("alpha entry should exist");
    let beta = registry.get("beta").expect("beta entry should exist");

    assert_eq!(alpha.state, PluginState::Initialized);
    assert_eq!(alpha.capabilities.len(), 1);
    assert_eq!(beta.state, PluginState::Failed);
    assert_eq!(
        beta.failure.as_deref(),
        Some("internal error: handshake failed")
    );
    assert!(
        bootstrap
            .coordinator
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.name == "tool.alpha")
    );
}

#[test]
fn bootstrap_rejects_duplicate_plugin_capabilities_deterministically() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initializer = FakeInitializer {
        responses: HashMap::from([
            (
                "alpha".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "alpha",
                    &["tool.shared"],
                    Arc::clone(&shutdowns),
                )),
            ),
            (
                "beta".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "beta",
                    &["tool.shared"],
                    Arc::clone(&shutdowns),
                )),
            ),
        ]),
    };

    let (bootstrap, _guard) =
        bootstrap_from(vec![manifest("beta"), manifest("alpha")], initializer);
    let snapshot = bootstrap.coordinator.plugin_registry().snapshot();
    let alpha = snapshot
        .iter()
        .find(|entry| entry.manifest.name == "alpha")
        .expect("alpha entry should exist");
    let beta = snapshot
        .iter()
        .find(|entry| entry.manifest.name == "beta")
        .expect("beta entry should exist");

    assert_eq!(alpha.state, PluginState::Initialized);
    assert_eq!(beta.state, PluginState::Failed);
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["beta".to_string()]
    );
}

#[tokio::test]
async fn bootstrap_integrates_plugin_declared_skills_into_runtime_catalog() {
    let _guard = TestEnvGuard::new();
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let mut plugin = loaded_plugin("alpha", &["tool.alpha"], shutdowns);
    plugin.declared_skills = vec![declared_skill("repo-search")];
    let initializer = FakeInitializer {
        responses: HashMap::from([("alpha".to_string(), FakePluginResponse::Loaded(plugin))]),
    };

    let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initializer)
        .await
        .expect("bootstrap should succeed");
    bootstrap.plugin_load_handle.wait_completed().await;
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = bootstrap
        .service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let items = bootstrap
        .service
        .list_composer_options(
            &session.session_id,
            ComposerOptionsRequest {
                query: Some("repo".to_string()),
                kinds: vec![ComposerOptionKind::Skill],
                limit: 10,
            },
        )
        .await
        .expect("composer options should load");

    assert!(items.iter().any(|item| item.id == "repo-search"));
}

#[tokio::test]
async fn bootstrap_background_load_syncs_governance_health_snapshot() {
    let _guard = TestEnvGuard::new();
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initializer = FakeInitializer {
        responses: HashMap::from([(
            "alpha".to_string(),
            FakePluginResponse::Loaded(loaded_plugin(
                "alpha",
                &["tool.alpha"],
                Arc::clone(&shutdowns),
            )),
        )]),
    };

    let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initializer)
        .await
        .expect("bootstrap should succeed");
    bootstrap.plugin_load_handle.wait_completed().await;

    let snapshot = bootstrap.governance.snapshot().await;
    let alpha = snapshot
        .plugins
        .into_iter()
        .find(|entry| entry.manifest.name == "alpha")
        .expect("alpha entry should exist");

    assert_eq!(alpha.health, PluginHealth::Healthy);
    assert!(alpha.last_checked_at.is_some());
}

#[test]
fn bootstrap_marks_plugin_failed_when_descriptor_is_invalid() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let mut invalid = capability("tool.invalid");
    invalid.kind = CapabilityKind::new("   ");
    let initializer = FakeInitializer {
        responses: HashMap::from([(
            "alpha".to_string(),
            FakePluginResponse::Loaded(LoadedPlugin {
                component: Arc::new(FakeManagedComponent {
                    name: "alpha".to_string(),
                    shutdowns: Arc::clone(&shutdowns),
                }),
                capabilities: vec![invalid.clone()],
                declared_skills: Vec::new(),
                contribution: crate::runtime_surface_assembler::RuntimeSurfaceContribution {
                    capability_invokers: vec![Arc::new(FakeCapabilityInvoker {
                        descriptor: invalid.clone(),
                    }) as Arc<dyn CapabilityInvoker>],
                    prompt_declarations: Vec::new(),
                    skills: Vec::new(),
                    hook_handlers: Vec::new(),
                },
            }),
        )]),
    };

    let (bootstrap, _guard) = bootstrap_from(vec![manifest("alpha")], initializer);
    let alpha = bootstrap
        .coordinator
        .plugin_registry()
        .get("alpha")
        .expect("alpha entry should exist");

    assert_eq!(alpha.state, PluginState::Failed);
    assert_eq!(alpha.health, PluginHealth::Unavailable);
    assert!(
        alpha
            .failure
            .as_deref()
            .is_some_and(|message| message.contains("invalid"))
    );
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["alpha".to_string()]
    );
}

#[tokio::test]
async fn governance_reload_swaps_runtime_surface_and_shutdowns_retired_plugins() {
    let _guard = TestEnvGuard::new();
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initial = FakeInitializer {
        responses: HashMap::from([(
            "alpha".to_string(),
            FakePluginResponse::Loaded(loaded_plugin(
                "alpha",
                &["tool.alpha"],
                Arc::clone(&shutdowns),
            )),
        )]),
    };
    let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initial)
        .await
        .expect("initial bootstrap should succeed");
    bootstrap.plugin_load_handle.wait_completed().await;

    let replacement = FakeInitializer {
        responses: HashMap::from([(
            "beta".to_string(),
            FakePluginResponse::Loaded(loaded_plugin(
                "beta",
                &["tool.beta"],
                Arc::clone(&shutdowns),
            )),
        )]),
    };
    let reload = bootstrap
        .governance
        .reload_from_manifests(
            vec![manifest("beta")],
            &replacement,
            vec![PathBuf::from("plugins")],
        )
        .await
        .expect("reload should succeed");

    assert_eq!(
        bootstrap
            .coordinator
            .plugin_registry()
            .snapshot()
            .into_iter()
            .map(|entry| entry.manifest.name)
            .collect::<Vec<_>>(),
        vec!["beta".to_string()]
    );
    assert_eq!(
        bootstrap
            .coordinator
            .capabilities()
            .into_iter()
            .map(|descriptor| descriptor.name)
            .filter(|name| name.starts_with("tool."))
            .collect::<Vec<_>>(),
        vec!["tool.beta".to_string()]
    );
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["alpha".to_string()]
    );
    assert_eq!(
        reload.snapshot.plugin_search_paths,
        vec![PathBuf::from("plugins")]
    );
}

#[test]
fn conflicting_capability_name_detects_existing_and_local_duplicates() {
    let registered = std::collections::HashSet::from(["tool.shared".to_string()]);
    assert_eq!(
        conflicting_capability_name(&registered, &[capability("tool.shared")]),
        Some("tool.shared".to_string())
    );
    assert_eq!(
        conflicting_capability_name(
            &std::collections::HashSet::new(),
            &[capability("tool.local"), capability("tool.local")]
        ),
        Some("tool.local".to_string())
    );
}

#[tokio::test]
async fn bootstrap_background_load_propagates_plugin_hook_handlers_into_agent_loop() {
    let _guard = TestEnvGuard::new();
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let mut plugin = loaded_plugin("alpha", &["tool.alpha"], shutdowns);
    plugin.contribution.hook_handlers = vec![Arc::new(BlockingPreCompactHook)];
    let initializer = FakeInitializer {
        responses: HashMap::from([("alpha".to_string(), FakePluginResponse::Loaded(plugin))]),
    };

    let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initializer)
        .await
        .expect("bootstrap should succeed");
    bootstrap.plugin_load_handle.wait_completed().await;

    let loop_ = bootstrap.service.current_loop().await;
    let state = astrcode_core::AgentState {
        session_id: "hook-session".to_string(),
        working_dir: std::env::temp_dir(),
        messages: vec![
            astrcode_core::LlmMessage::User {
                content: "turn-1".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            },
            astrcode_core::LlmMessage::Assistant {
                content: "reply-1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            astrcode_core::LlmMessage::User {
                content: "turn-2".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            },
        ],
        phase: astrcode_core::Phase::Thinking,
        turn_count: 2,
    };

    let error = loop_
        .manual_compact_event(
            &state,
            astrcode_runtime_agent_loop::CompactionTailSnapshot::from_messages(&state.messages, 1),
            None,
        )
        .await
        .expect_err("plugin hook should block manual compact");

    assert!(error.to_string().contains("blocked by plugin hook"));
}
