//! # 运行时协调器
//!
//! 统一管理运行时实例、插件注册表和可用能力列表。
//!
//! ## 职责
//!
//! - 持有当前活跃的运行时句柄（`RuntimeHandle`）
//! - 管理插件注册表快照
//! - 维护可用能力描述符列表
//! - 管理可关闭的子组件列表
//! - 提供原子化的运行时表面替换（`replace_runtime_surface`）

use std::sync::{Arc, RwLock};

use crate::{
    AstrError, CapabilitySpec, ManagedRuntimeComponent, PluginRegistry, Result, RuntimeHandle,
    plugin::PluginEntry, support,
};

/// 运行时协调器。
///
/// 作为运行时的统一门面，管理运行时句柄、插件注册表、能力列表
/// 和可关闭子组件的生命周期。
///
/// ## 设计要点
///
/// - 通过 `replace_runtime_surface` 实现原子化的运行时表面替换， 用于插件热重载或运行时切换场景
/// - 关闭时按确定顺序先停止运行时，再逐个关闭托管组件
pub struct RuntimeCoordinator {
    /// 当前活跃的运行时句柄
    active_runtime: Arc<dyn RuntimeHandle>,
    /// 插件注册表，管理插件生命周期和健康状态
    plugin_registry: Arc<PluginRegistry>,
    /// 可用能力描述符列表（原子引用，支持并发读取）
    capabilities: RwLock<Arc<[CapabilitySpec]>>,
    /// 可关闭的托管组件列表，按注册顺序关闭
    managed_components: RwLock<Vec<Arc<dyn ManagedRuntimeComponent>>>,
}

impl std::fmt::Debug for RuntimeCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeCoordinator")
            .field("runtime_name", &self.active_runtime.runtime_name())
            .field("runtime_kind", &self.active_runtime.runtime_kind())
            .finish_non_exhaustive()
    }
}

impl RuntimeCoordinator {
    /// 创建运行时协调器。
    pub fn new(
        active_runtime: Arc<dyn RuntimeHandle>,
        plugin_registry: Arc<PluginRegistry>,
        capabilities: Vec<CapabilitySpec>,
    ) -> Self {
        Self {
            active_runtime,
            plugin_registry,
            capabilities: RwLock::new(Arc::from(capabilities)),
            managed_components: RwLock::new(Vec::new()),
        }
    }

    /// 设置托管组件列表。
    ///
    /// 采用 builder 风格的链式调用，组件将在 `shutdown` 时按顺序关闭。
    pub fn with_managed_components(
        self,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Self {
        support::with_write_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            |components| *components = managed_components,
        );
        self
    }

    /// 获取当前运行时句柄的克隆引用。
    pub fn runtime(&self) -> Arc<dyn RuntimeHandle> {
        Arc::clone(&self.active_runtime)
    }

    /// 获取插件注册表的克隆引用。
    pub fn plugin_registry(&self) -> Arc<PluginRegistry> {
        Arc::clone(&self.plugin_registry)
    }

    /// 获取当前可用能力描述符列表的副本。
    pub fn capabilities(&self) -> Vec<CapabilitySpec> {
        support::with_read_lock_recovery(
            &self.capabilities,
            "runtime coordinator capabilities",
            |capabilities| capabilities.iter().cloned().collect(),
        )
    }

    pub fn managed_components(&self) -> Vec<Arc<dyn ManagedRuntimeComponent>> {
        support::with_read_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            Clone::clone,
        )
    }

    /// 原子替换运行时表面（插件热重载核心方法）。
    ///
    /// 一次性替换三样东西：插件注册表快照、能力描述符列表、托管组件列表。
    /// 返回旧的托管组件列表，调用方负责逐个关闭它们。
    ///
    /// 为什么需要原子替换：如果逐项更新，中间状态会导致：
    /// - 新插件已注册但旧能力描述符还在 → 路由找不到能力
    /// - 旧插件已清空但旧组件还在引用 → 悬垂引用
    pub fn replace_runtime_surface(
        &self,
        plugin_entries: Vec<PluginEntry>,
        capabilities: Vec<CapabilitySpec>,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Vec<Arc<dyn ManagedRuntimeComponent>> {
        self.plugin_registry.replace_snapshot(plugin_entries);
        support::with_write_lock_recovery(
            &self.capabilities,
            "runtime coordinator capabilities",
            |current_capabilities| *current_capabilities = Arc::from(capabilities),
        );
        support::with_write_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            |current_components| std::mem::replace(current_components, managed_components),
        )
    }

    /// 关闭运行时和所有托管组件。
    ///
    /// 关闭顺序是确定性的：先关闭运行时句柄（停止接收新请求），
    /// 再逐个关闭托管组件（释放资源）。所有失败会被收集并合并
    /// 为单个错误返回——即使某个组件关闭失败，仍会尝试关闭剩余组件。
    pub async fn shutdown(&self, timeout_secs: u64) -> Result<()> {
        let mut failures = Vec::new();

        if let Err(error) = self.active_runtime.shutdown(timeout_secs).await {
            log::error!(
                "failed to shut down runtime '{}' (kind '{}'): {}",
                self.active_runtime.runtime_name(),
                self.active_runtime.runtime_kind(),
                error
            );
            failures.push(format!(
                "runtime '{}' failed to shut down: {}",
                self.active_runtime.runtime_name(),
                error
            ));
        }

        // Keep the shutdown order deterministic so tests and operational logs can explain
        // exactly which managed component was closed after the runtime stopped accepting work.
        let managed_components = support::with_read_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            Clone::clone,
        );

        for component in managed_components {
            if let Err(error) = component.shutdown_component().await {
                let component_name = component.component_name();
                log::error!(
                    "failed to shut down managed component '{}': {}",
                    component_name,
                    error
                );
                failures.push(format!(
                    "managed component '{}' failed to shut down: {}",
                    component_name, error
                ));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(AstrError::Internal(failures.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use super::RuntimeCoordinator;
    use crate::{
        AstrError, CapabilityKind, CapabilitySpec, InvocationMode, ManagedRuntimeComponent,
        PluginRegistry, Result, RuntimeHandle, SideEffect, Stability,
        plugin::{PluginEntry, PluginHealth},
    };

    struct FakeRuntimeHandle {
        events: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    #[async_trait]
    impl RuntimeHandle for FakeRuntimeHandle {
        fn runtime_name(&self) -> &'static str {
            "test-runtime"
        }

        fn runtime_kind(&self) -> &'static str {
            "unit-test"
        }

        async fn shutdown(&self, _timeout_secs: u64) -> Result<()> {
            self.events
                .lock()
                .expect("events lock")
                .push("runtime".to_string());
            if self.fail {
                Err(AstrError::Internal("runtime failure".to_string()))
            } else {
                Ok(())
            }
        }
    }

    struct FakeManagedComponent {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    #[async_trait]
    impl ManagedRuntimeComponent for FakeManagedComponent {
        fn component_name(&self) -> String {
            self.name.to_string()
        }

        async fn shutdown_component(&self) -> Result<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(self.name.to_string());
            if self.fail {
                Err(AstrError::Internal(format!("{} failure", self.name)))
            } else {
                Ok(())
            }
        }
    }

    fn capability(name: &str) -> CapabilitySpec {
        CapabilitySpec {
            name: name.into(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffect::None,
            stability: Stability::Stable,
            metadata: json!(null),
            max_result_inline_size: None,
        }
    }

    #[tokio::test]
    async fn shutdown_runs_runtime_before_managed_components() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::new(PluginRegistry::default()),
            vec![capability("tool.sample")],
        )
        .with_managed_components(vec![
            Arc::new(FakeManagedComponent {
                name: "plugin-a",
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::new(FakeManagedComponent {
                name: "plugin-b",
                events: Arc::clone(&events),
                fail: false,
            }),
        ]);

        coordinator.shutdown(3).await.expect("shutdown should pass");

        assert_eq!(
            events.lock().expect("events lock").clone(),
            vec![
                "runtime".to_string(),
                "plugin-a".to_string(),
                "plugin-b".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn shutdown_collects_failures_after_attempting_every_component() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: true,
            }),
            Arc::new(PluginRegistry::default()),
            vec![capability("tool.sample")],
        )
        .with_managed_components(vec![
            Arc::new(FakeManagedComponent {
                name: "plugin-a",
                events: Arc::clone(&events),
                fail: true,
            }),
            Arc::new(FakeManagedComponent {
                name: "plugin-b",
                events,
                fail: false,
            }),
        ]);

        let error = coordinator
            .shutdown(3)
            .await
            .expect_err("shutdown should bubble failure");

        let message = error.to_string();
        assert!(message.contains("runtime 'test-runtime' failed to shut down"));
        assert!(message.contains("managed component 'plugin-a' failed to shut down"));
    }

    #[test]
    fn replace_runtime_surface_swaps_registry_capabilities_and_components() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(PluginRegistry::default());
        registry.record_discovered(crate::PluginManifest {
            name: "alpha".to_string(),
            version: "0.1.0".to_string(),
            description: "alpha".to_string(),
            plugin_type: vec![crate::PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("alpha.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
        });
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::clone(&registry),
            vec![capability("tool.alpha")],
        )
        .with_managed_components(vec![Arc::new(FakeManagedComponent {
            name: "plugin-a",
            events,
            fail: false,
        })]);

        let old = coordinator.replace_runtime_surface(
            vec![PluginEntry {
                manifest: crate::PluginManifest {
                    name: "beta".to_string(),
                    version: "0.2.0".to_string(),
                    description: "beta".to_string(),
                    plugin_type: vec![crate::PluginType::Tool],
                    capabilities: Vec::new(),
                    executable: Some("beta.exe".to_string()),
                    args: Vec::new(),
                    working_dir: None,
                    repository: None,
                },
                state: crate::PluginState::Initialized,
                health: PluginHealth::Healthy,
                failure_count: 0,
                capabilities: vec![capability("tool.beta")],
                failure: None,
                warnings: Vec::new(),
                last_checked_at: None,
            }],
            vec![capability("tool.beta")],
            Vec::new(),
        );

        assert_eq!(old.len(), 1);
        assert_eq!(
            coordinator
                .plugin_registry()
                .snapshot()
                .into_iter()
                .map(|entry| entry.manifest.name)
                .collect::<Vec<_>>(),
            vec!["beta".to_string()]
        );
        assert_eq!(
            coordinator
                .capabilities()
                .into_iter()
                .map(|descriptor| descriptor.name.to_string())
                .collect::<Vec<_>>(),
            vec!["tool.beta".to_string()]
        );
    }
}
