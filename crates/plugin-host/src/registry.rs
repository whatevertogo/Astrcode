use std::{collections::BTreeMap, sync::RwLock};

use astrcode_core::{CapabilitySpec, Result};

use crate::{
    PluginActiveSnapshot, PluginDescriptor, PluginManifest, descriptor::validate_descriptors,
};

/// 插件生命周期状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginState {
    Discovered,
    Initialized,
    Failed,
}

/// 插件健康状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginHealth {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

/// 插件注册表条目。
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub health: PluginHealth,
    pub failure_count: u32,
    pub capabilities: Vec<CapabilitySpec>,
    pub failure: Option<String>,
    pub warnings: Vec<String>,
    pub last_checked_at: Option<String>,
}

/// plugin-host 的最小注册表。
///
/// 第一阶段只负责三件事：
/// - 接收一组 descriptor 并构建 candidate snapshot
/// - 原子提交 candidate 成为 active snapshot
/// - 在提交失败或显式放弃时回滚 candidate
///
/// 发现、进程管理和健康检查会在后续阶段继续补进来，
/// 但不应该重新把 registry 扩成旧 `core::plugin::registry` 那种生命周期大全。
#[derive(Debug, Default)]
pub struct PluginRegistry {
    state: RwLock<PluginRegistryState>,
}

#[derive(Debug, Default)]
struct PluginRegistryState {
    next_revision: u64,
    active: Option<PluginActiveSnapshot>,
    candidate: Option<PluginActiveSnapshot>,
    plugins: BTreeMap<String, PluginEntry>,
}

impl PluginRegistry {
    /// 读取当前 active snapshot。
    pub fn active_snapshot(&self) -> Option<PluginActiveSnapshot> {
        self.state
            .read()
            .expect("plugin registry lock poisoned")
            .active
            .clone()
    }

    /// 读取当前 candidate snapshot。
    pub fn candidate_snapshot(&self) -> Option<PluginActiveSnapshot> {
        self.state
            .read()
            .expect("plugin registry lock poisoned")
            .candidate
            .clone()
    }

    /// 使用给定 descriptors 构建下一版 candidate snapshot。
    ///
    /// candidate 只对后续 commit 生效，不影响当前 active turn。
    pub fn stage_candidate(
        &self,
        descriptors: impl IntoIterator<Item = PluginDescriptor>,
    ) -> Result<PluginActiveSnapshot> {
        let mut state = self.state.write().expect("plugin registry lock poisoned");
        state.next_revision = state.next_revision.saturating_add(1);
        let descriptors = descriptors.into_iter().collect::<Vec<_>>();
        validate_descriptors(&descriptors)?;
        let snapshot = PluginActiveSnapshot::from_descriptors(
            state.next_revision,
            format!("plugin-snapshot-{}", state.next_revision),
            &descriptors,
        );
        state.candidate = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// 提交 candidate snapshot。
    ///
    /// 成功后 active 被替换，candidate 被清空。
    pub fn commit_candidate(&self) -> Option<PluginActiveSnapshot> {
        let mut state = self.state.write().expect("plugin registry lock poisoned");
        let candidate = state.candidate.take()?;
        state.active = Some(candidate.clone());
        Some(candidate)
    }

    /// 丢弃当前 candidate snapshot。
    pub fn rollback_candidate(&self) -> Option<PluginActiveSnapshot> {
        self.state
            .write()
            .expect("plugin registry lock poisoned")
            .candidate
            .take()
    }

    /// 直接替换 active snapshot。
    ///
    /// 这个入口保留给测试和后续 reload 恢复流程使用，
    /// 避免 host 在回放持久化状态时必须走 staging 再 commit。
    pub fn replace_active(&self, snapshot: PluginActiveSnapshot) {
        let mut state = self.state.write().expect("plugin registry lock poisoned");
        state.next_revision = state.next_revision.max(snapshot.revision);
        state.active = Some(snapshot);
        state.candidate = None;
    }

    /// 记录一个新发现的插件。
    pub fn record_discovered(&self, manifest: PluginManifest) {
        self.upsert_plugin(PluginEntry {
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
    pub fn record_initialized(
        &self,
        manifest: PluginManifest,
        capabilities: Vec<CapabilitySpec>,
        warnings: Vec<String>,
    ) {
        self.upsert_plugin(PluginEntry {
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
    pub fn record_failed(
        &self,
        manifest: PluginManifest,
        failure: impl Into<String>,
        capabilities: Vec<CapabilitySpec>,
        warnings: Vec<String>,
    ) {
        self.upsert_plugin(PluginEntry {
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
    pub fn get(&self, name: &str) -> Option<PluginEntry> {
        self.state
            .read()
            .expect("plugin registry lock poisoned")
            .plugins
            .get(name)
            .cloned()
    }

    /// 获取所有插件条目的快照。
    pub fn snapshot(&self) -> Vec<PluginEntry> {
        self.state
            .read()
            .expect("plugin registry lock poisoned")
            .plugins
            .values()
            .cloned()
            .collect()
    }

    /// 原子替换整个插件生命周期快照。
    pub fn replace_snapshot(&self, entries: Vec<PluginEntry>) {
        let mut state = self.state.write().expect("plugin registry lock poisoned");
        state.plugins.clear();
        for entry in entries {
            state.plugins.insert(entry.manifest.name.clone(), entry);
        }
    }

    /// 记录插件运行时成功事件。
    pub fn record_runtime_success(&self, name: &str, checked_at: String) {
        self.mutate_plugin(name, |entry| {
            if entry.state == PluginState::Initialized {
                entry.health = PluginHealth::Healthy;
            }
            entry.failure_count = 0;
            entry.failure = None;
            entry.last_checked_at = Some(checked_at);
        });
    }

    /// 记录插件运行时失败事件。
    pub fn record_runtime_failure(
        &self,
        name: &str,
        failure: impl Into<String>,
        checked_at: String,
    ) {
        let failure = failure.into();
        self.mutate_plugin(name, |entry| {
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
    pub fn record_health_probe(
        &self,
        name: &str,
        health: PluginHealth,
        failure: Option<String>,
        checked_at: String,
    ) {
        self.mutate_plugin(name, |entry| {
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

    fn upsert_plugin(&self, entry: PluginEntry) {
        self.state
            .write()
            .expect("plugin registry lock poisoned")
            .plugins
            .insert(entry.manifest.name.clone(), entry);
    }

    fn mutate_plugin(&self, name: &str, update: impl FnOnce(&mut PluginEntry)) {
        if let Some(entry) = self
            .state
            .write()
            .expect("plugin registry lock poisoned")
            .plugins
            .get_mut(name)
        {
            update(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};

    use super::PluginRegistry;
    use crate::{PluginDescriptor, descriptor::PluginSourceKind};

    fn capability(name: &str) -> CapabilitySpec {
        CapabilitySpec {
            name: name.into(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: Default::default(),
            output_schema: Default::default(),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffect::None,
            stability: Stability::Stable,
            metadata: Default::default(),
            max_result_inline_size: None,
        }
    }

    fn builtin(plugin_id: &str, tool_name: &str) -> PluginDescriptor {
        let mut descriptor = PluginDescriptor::builtin(plugin_id, format!("{plugin_id} display"));
        descriptor.source_kind = PluginSourceKind::Builtin;
        descriptor.tools.push(capability(tool_name));
        descriptor
    }

    #[test]
    fn stage_candidate_does_not_replace_active_snapshot() {
        let registry = PluginRegistry::default();

        let staged = registry
            .stage_candidate(vec![builtin("alpha", "tool.alpha")])
            .expect("candidate should stage");

        assert_eq!(staged.revision, 1);
        assert!(registry.active_snapshot().is_none());
        assert_eq!(
            registry
                .candidate_snapshot()
                .expect("candidate should exist")
                .plugin_ids,
            vec!["alpha".to_string()]
        );
    }

    #[test]
    fn commit_candidate_promotes_snapshot_and_clears_candidate() {
        let registry = PluginRegistry::default();
        registry
            .stage_candidate(vec![builtin("alpha", "tool.alpha")])
            .expect("candidate should stage");

        let committed = registry
            .commit_candidate()
            .expect("candidate should commit");

        assert_eq!(committed.revision, 1);
        assert!(registry.candidate_snapshot().is_none());
        assert_eq!(
            registry
                .active_snapshot()
                .expect("active snapshot should exist")
                .tools
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>(),
            vec!["tool.alpha".to_string()]
        );
    }

    #[test]
    fn rollback_candidate_preserves_previous_active_snapshot() {
        let registry = PluginRegistry::default();
        registry
            .stage_candidate(vec![builtin("alpha", "tool.alpha")])
            .expect("first candidate should stage");
        let active = registry
            .commit_candidate()
            .expect("first candidate should commit");

        registry
            .stage_candidate(vec![builtin("beta", "tool.beta")])
            .expect("second candidate should stage");
        let rolled_back = registry
            .rollback_candidate()
            .expect("candidate should roll back");

        assert_eq!(rolled_back.revision, 2);
        assert_eq!(
            registry
                .active_snapshot()
                .expect("active snapshot should be preserved")
                .plugin_ids,
            active.plugin_ids
        );
        assert!(registry.candidate_snapshot().is_none());
    }

    #[test]
    fn stage_candidate_rejects_invalid_descriptor_sets() {
        let registry = PluginRegistry::default();
        let duplicate = vec![
            builtin("alpha", "tool.alpha"),
            builtin("alpha", "tool.beta"),
        ];

        let error = registry
            .stage_candidate(duplicate)
            .expect_err("duplicate plugin ids should fail");

        assert!(error.to_string().contains("plugin_id 'alpha' 重复"));
        assert!(registry.active_snapshot().is_none());
        assert!(registry.candidate_snapshot().is_none());
    }
}
