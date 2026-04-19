//! # 插件注册表
//!
//! 管理已发现插件的生命周期状态、健康检查和能力声明。
//!
//! ## 设计要点
//!
//! - 使用 `RwLock<BTreeMap>` 保证线程安全和有序遍历
//! - 插件状态机：`Discovered` → `Initialized` / `Failed`
//! - 健康状态独立于生命周期状态（`Healthy` / `Degraded` / `Unavailable`）
//! - 支持运行时快照替换（用于插件热重载场景）

use std::{collections::BTreeMap, sync::RwLock};

use crate::{CapabilitySpec, PluginManifest};

/// 插件生命周期状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginState {
    /// 已发现（清单已加载，但尚未初始化）
    Discovered,
    /// 已初始化（能力已注册，可以正常调用）
    Initialized,
    /// 初始化失败
    Failed,
}

/// 插件健康状态。
///
/// 与 `PluginState` 不同，健康状态反映运行时状况，
/// 一个已初始化的插件可能因网络问题变为 `Degraded`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginHealth {
    /// 尚未检查
    Unknown,
    /// 正常运行
    Healthy,
    /// 部分功能异常
    Degraded,
    /// 不可用
    Unavailable,
}

/// 插件注册表条目。
///
/// 包含插件的完整运行时状态：清单、生命周期状态、健康状态、失败记录等。
#[derive(Debug, Clone)]
pub struct PluginEntry {
    /// 插件清单
    pub manifest: PluginManifest,
    /// 生命周期状态
    pub state: PluginState,
    /// 健康状态
    pub health: PluginHealth,
    /// 连续失败次数
    pub failure_count: u32,
    /// 已注册的能力列表
    pub capabilities: Vec<CapabilitySpec>,
    /// 失败原因（仅在失败时设置）
    pub failure: Option<String>,
    /// 非致命诊断信息（如 skill 物化失败、allowed_tools 被降级）。
    ///
    /// 这些 warning 不会改变插件主状态；它们用于把“插件已加载但部分能力
    /// 或资源需要人工关注”的事实显式暴露给上层 UI，而不是静默吞掉。
    pub warnings: Vec<String>,
    /// 最后一次健康检查时间
    pub last_checked_at: Option<String>,
}

/// 插件注册表。
///
/// 线程安全的插件状态存储，支持并发读写。
/// 使用 `RwLock` 而非 `Mutex` 因为读操作远多于写操作。
#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: RwLock<BTreeMap<String, PluginEntry>>,
}

impl PluginRegistry {
    /// 记录一个新发现的插件。
    ///
    /// 如果同名插件已存在，会被覆盖。
    pub fn record_discovered(&self, manifest: PluginManifest) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Discovered,
            health: PluginHealth::Unknown,
            failure_count: 0,
            capabilities: Vec::new(),
            failure: None,
            warnings: Vec::new(),
            last_checked_at: None,
        });
    }

    /// 记录插件初始化成功，将状态推进到 `Initialized`。
    ///
    /// 初始化成功后健康状态重置为 `Healthy`，失败计数清零。
    pub fn record_initialized(
        &self,
        manifest: PluginManifest,
        capabilities: Vec<CapabilitySpec>,
        warnings: Vec<String>,
    ) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities,
            failure: None,
            warnings,
            last_checked_at: None,
        });
    }

    /// 记录插件初始化失败，将状态标记为 `Failed`。
    ///
    /// 失败后健康状态设为 `Unavailable`，并记录失败原因。
    pub fn record_failed(
        &self,
        manifest: PluginManifest,
        failure: impl Into<String>,
        capabilities: Vec<CapabilitySpec>,
        warnings: Vec<String>,
    ) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Failed,
            health: PluginHealth::Unavailable,
            failure_count: 1,
            capabilities,
            failure: Some(failure.into()),
            warnings,
            last_checked_at: None,
        });
    }

    /// 按名称查询插件条目。
    ///
    /// 返回 `None` 表示该插件尚未被发现或已从注册表中移除。
    pub fn get(&self, name: &str) -> Option<PluginEntry> {
        self.plugins
            .read()
            .expect("plugin registry lock poisoned")
            .get(name)
            .cloned()
    }

    /// 获取所有插件条目的快照。
    ///
    /// 返回当前注册表中所有插件的副本，调用方持有快照后
    /// 注册表的后续变更不会影响已返回的快照。
    pub fn snapshot(&self) -> Vec<PluginEntry> {
        self.plugins
            .read()
            .expect("plugin registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 原子替换整个插件注册表快照。
    ///
    /// 用于插件热重载场景：新插件集合一次性替换旧集合，
    /// 避免逐条更新导致中间状态不一致。
    pub fn replace_snapshot(&self, entries: Vec<PluginEntry>) {
        let mut plugins = self.plugins.write().expect("plugin registry lock poisoned");
        plugins.clear();
        for entry in entries {
            plugins.insert(entry.manifest.name.clone(), entry);
        }
    }

    /// 记录插件运行时成功事件。
    ///
    /// 将健康状态重置为 `Healthy` 并清零失败计数，
    /// 表明插件当前运行正常。
    pub fn record_runtime_success(&self, name: &str, checked_at: String) {
        self.mutate(name, |entry| {
            if entry.state == PluginState::Initialized {
                entry.health = PluginHealth::Healthy;
            }
            entry.failure_count = 0;
            entry.failure = None;
            entry.last_checked_at = Some(checked_at);
        });
    }

    /// 记录插件运行时失败事件。
    ///
    /// 实现渐进式健康度评估：
    /// - 1~2 次失败 → `Degraded`（降级但不完全禁用，后续成功可恢复）
    /// - 3 次及以上 → `Unavailable`（完全禁用，需要人工或自动恢复机制介入）
    /// - 非 Initialized 状态的插件 → 直接 `Unavailable`
    pub fn record_runtime_failure(
        &self,
        name: &str,
        failure: impl Into<String>,
        checked_at: String,
    ) {
        let failure = failure.into();
        self.mutate(name, |entry| {
            entry.failure_count = entry.failure_count.saturating_add(1);
            entry.failure = Some(failure.clone());
            entry.last_checked_at = Some(checked_at);
            if entry.state == PluginState::Initialized {
                entry.health = if entry.failure_count >= 3 {
                    PluginHealth::Unavailable
                } else {
                    PluginHealth::Degraded
                };
            } else {
                entry.health = PluginHealth::Unavailable;
            }
        });
    }

    /// 记录一次主动健康探测结果。
    ///
    /// 与 `record_runtime_success/failure` 不同，此方法允许调用方
    /// 直接指定健康状态，适用于自定义健康检查逻辑。
    pub fn record_health_probe(
        &self,
        name: &str,
        health: PluginHealth,
        failure: Option<String>,
        checked_at: String,
    ) {
        self.mutate(name, |entry| {
            entry.health = health.clone();
            if matches!(health, PluginHealth::Healthy) {
                entry.failure_count = 0;
                entry.failure = None;
            } else if let Some(message) = failure.clone() {
                entry.failure = Some(message);
            }
            entry.last_checked_at = Some(checked_at.clone());
        });
    }

    fn upsert(&self, entry: PluginEntry) {
        self.plugins
            .write()
            .expect("plugin registry lock poisoned")
            .insert(entry.manifest.name.clone(), entry);
    }

    fn mutate(&self, name: &str, update: impl FnOnce(&mut PluginEntry)) {
        if let Some(entry) = self
            .plugins
            .write()
            .expect("plugin registry lock poisoned")
            .get_mut(name)
        {
            update(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{PluginHealth, PluginRegistry, PluginState};
    use crate::{
        CapabilityKind, CapabilitySpec, InvocationMode, PluginManifest, PluginType, SideEffect,
        Stability,
    };

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: format!("{name} manifest"),
            plugin_type: vec![PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("plugin.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
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

    #[test]
    fn records_state_transitions_and_failure_details() {
        let registry = PluginRegistry::default();
        let manifest = manifest("repo-inspector");

        registry.record_discovered(manifest.clone());
        assert_eq!(
            registry
                .get("repo-inspector")
                .expect("entry should exist")
                .state,
            PluginState::Discovered
        );

        registry.record_initialized(
            manifest.clone(),
            vec![capability("tool.repo.inspect")],
            vec!["skill warning".to_string()],
        );
        let initialized = registry
            .get("repo-inspector")
            .expect("initialized entry should exist");
        assert_eq!(initialized.state, PluginState::Initialized);
        assert_eq!(initialized.health, PluginHealth::Healthy);
        assert_eq!(initialized.capabilities.len(), 1);
        assert!(initialized.failure.is_none());
        assert_eq!(initialized.warnings, vec!["skill warning".to_string()]);

        registry.record_failed(
            manifest,
            "capability conflict",
            vec![capability("tool.repo.inspect")],
            vec!["materialize failed".to_string()],
        );
        let failed = registry
            .get("repo-inspector")
            .expect("failed entry should exist");
        assert_eq!(failed.state, PluginState::Failed);
        assert_eq!(failed.health, PluginHealth::Unavailable);
        assert_eq!(failed.failure.as_deref(), Some("capability conflict"));
        assert_eq!(failed.capabilities.len(), 1);
        assert_eq!(failed.warnings, vec!["materialize failed".to_string()]);
    }

    #[test]
    fn snapshot_is_sorted_by_plugin_name() {
        let registry = PluginRegistry::default();
        registry.record_discovered(manifest("zeta"));
        registry.record_discovered(manifest("alpha"));

        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot
                .into_iter()
                .map(|entry| entry.manifest.name)
                .collect::<Vec<_>>(),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[test]
    fn replace_snapshot_overwrites_existing_entries() {
        let registry = PluginRegistry::default();
        registry.record_discovered(manifest("alpha"));
        registry.replace_snapshot(vec![super::PluginEntry {
            manifest: manifest("beta"),
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities: vec![capability("tool.beta")],
            failure: None,
            warnings: Vec::new(),
            last_checked_at: None,
        }]);

        assert!(registry.get("alpha").is_none());
        assert_eq!(
            registry.get("beta").expect("beta should exist").state,
            PluginState::Initialized
        );
    }

    #[test]
    fn runtime_health_transitions_degrade_then_recover() {
        let registry = PluginRegistry::default();
        registry.record_initialized(
            manifest("alpha"),
            vec![capability("tool.alpha")],
            Vec::new(),
        );

        registry.record_runtime_failure("alpha", "transport closed", Utc::now().to_rfc3339());
        let degraded = registry.get("alpha").expect("alpha should exist");
        assert_eq!(degraded.health, PluginHealth::Degraded);
        assert_eq!(degraded.failure_count, 1);

        registry.record_runtime_success("alpha", Utc::now().to_rfc3339());
        let healthy = registry.get("alpha").expect("alpha should still exist");
        assert_eq!(healthy.health, PluginHealth::Healthy);
        assert_eq!(healthy.failure_count, 0);
        assert!(healthy.failure.is_none());
    }
}
