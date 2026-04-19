//! Working-dir 级 agent profile 解析与缓存。
//!
//! `application` 负责解析 profile 并缓存结果，`session-runtime` 只消费已解析输入。
//! 缓存键使用规范化路径，并预留显式失效入口防止 working-dir 变更后读到旧值。

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use astrcode_core::AgentProfile;
use dashmap::DashMap;

use crate::errors::ApplicationError;

/// Agent profile 加载端口。
///
/// `adapter-agents` 提供具体实现，`application` 通过此 trait 解耦。
/// 为什么不直接依赖 adapter-agents：architecture constraint 要求 application
/// 不依赖任何 adapter-*，只依赖 core + kernel + session-runtime。
pub trait ProfileProvider: Send + Sync {
    /// 加载指定 working-dir 可见的全部 agent profiles。
    ///
    /// 优先级由实现决定（builtin → 用户级 → 项目级），application 不关心细节。
    fn load_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> Result<Vec<AgentProfile>, ApplicationError>;

    /// 加载不绑定项目 scope 的全局 agents。
    fn load_global(&self) -> Result<Vec<AgentProfile>, ApplicationError>;
}

/// 基于 working-dir 的 profile 解析与缓存服务。
///
/// - 首次请求：委托 `ProfileProvider` 加载并缓存
/// - 再次请求（同一路径）：直接返回缓存
/// - 缓存不能替代业务校验：命中缓存但 agent 不存在仍返回错误
pub struct ProfileResolutionService {
    provider: Arc<dyn ProfileProvider>,
    /// 缓存键为规范化后的 working-dir canonical path
    cache: DashMap<PathBuf, Arc<Vec<AgentProfile>>>,
    /// 全局 profile 缓存（不绑定 working-dir）
    global_cache: RwLock<Option<Arc<Vec<AgentProfile>>>>,
}

impl ProfileResolutionService {
    pub fn new(provider: Arc<dyn ProfileProvider>) -> Self {
        Self {
            provider,
            cache: DashMap::new(),
            global_cache: RwLock::new(None),
        }
    }

    /// 指定 working-dir 可见的 profile 列表。
    ///
    /// 路径规范化后作为缓存键；首次访问委托 provider 加载。
    pub fn resolve(&self, working_dir: &Path) -> Result<Arc<Vec<AgentProfile>>, ApplicationError> {
        let canonical = working_dir_canonical(working_dir);

        if let Some(entry) = self.cache.get(&canonical) {
            return Ok(Arc::clone(entry.value()));
        }

        let profiles = self
            .provider
            .load_for_working_dir(&canonical)
            .map(Arc::new)?;

        self.cache.insert(canonical, Arc::clone(&profiles));
        Ok(profiles)
    }

    /// 全局 profile 列表（不绑定 working-dir）。
    pub fn resolve_global(&self) -> Result<Arc<Vec<AgentProfile>>, ApplicationError> {
        {
            let guard = self.read_global_cache();
            if let Some(ref cached) = *guard {
                return Ok(Arc::clone(cached));
            }
        }

        let profiles = self.provider.load_global().map(Arc::new)?;

        {
            let mut guard = self.write_global_cache();
            *guard = Some(Arc::clone(&profiles));
        }

        Ok(profiles)
    }

    /// 按 profile ID 查找指定 working-dir 可见的 profile。
    ///
    /// 即使缓存命中，profile 不存在仍返回 NotFound。
    pub fn find_profile(
        &self,
        working_dir: &Path,
        profile_id: &str,
    ) -> Result<AgentProfile, ApplicationError> {
        let profiles = self.resolve(working_dir)?;
        profiles
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or_else(|| {
                ApplicationError::NotFound(format!(
                    "agent profile '{}' not found for working-dir '{}'",
                    profile_id,
                    working_dir.display()
                ))
            })
    }

    /// 按 profile ID 查找全局 profile。
    pub fn find_global_profile(&self, profile_id: &str) -> Result<AgentProfile, ApplicationError> {
        let profiles = self.resolve_global()?;
        profiles
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or_else(|| {
                ApplicationError::NotFound(format!(
                    "global agent profile '{}' not found",
                    profile_id
                ))
            })
    }

    /// 使指定 working-dir 的缓存失效。
    ///
    /// 文件监听检测到 agent 定义变更时调用。
    pub fn invalidate(&self, working_dir: &Path) {
        let canonical = working_dir_canonical(working_dir);
        self.cache.remove(&canonical);
    }

    /// 使全局缓存失效。
    pub fn invalidate_global(&self) {
        self.cache.clear();
        let mut guard = self.write_global_cache();
        *guard = None;
    }

    /// 使所有缓存失效。
    pub fn invalidate_all(&self) {
        self.cache.clear();
        self.invalidate_global();
    }

    fn read_global_cache(&self) -> RwLockReadGuard<'_, Option<Arc<Vec<AgentProfile>>>> {
        self.global_cache
            .read()
            .expect("global profile cache lock should not be poisoned")
    }

    fn write_global_cache(&self) -> RwLockWriteGuard<'_, Option<Arc<Vec<AgentProfile>>>> {
        self.global_cache
            .write()
            .expect("global profile cache lock should not be poisoned")
    }
}

/// 规范化路径用于缓存键。
///
/// 使用 `canonicalize` 获取绝对路径；如果路径不存在（尚未创建的工作目录），
/// 回退到 `PathBuf::canonicalize` 失败前的原始路径。
fn working_dir_canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    /// 测试用 stub provider，记录调用次数以验证缓存行为。
    struct StubProfileProvider {
        profiles: Vec<AgentProfile>,
        load_call_count: AtomicUsize,
        global_call_count: AtomicUsize,
    }

    impl StubProfileProvider {
        fn new(profiles: Vec<AgentProfile>) -> Self {
            Self {
                profiles,
                load_call_count: AtomicUsize::new(0),
                global_call_count: AtomicUsize::new(0),
            }
        }

        fn load_count(&self) -> usize {
            self.load_call_count.load(Ordering::SeqCst)
        }

        fn global_count(&self) -> usize {
            self.global_call_count.load(Ordering::SeqCst)
        }
    }

    impl ProfileProvider for StubProfileProvider {
        fn load_for_working_dir(
            &self,
            _working_dir: &Path,
        ) -> Result<Vec<AgentProfile>, ApplicationError> {
            self.load_call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self.profiles.clone())
        }

        fn load_global(&self) -> Result<Vec<AgentProfile>, ApplicationError> {
            self.global_call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self.profiles.clone())
        }
    }

    struct MutableProfileProvider {
        profiles: Mutex<Vec<AgentProfile>>,
        load_call_count: AtomicUsize,
        global_call_count: AtomicUsize,
    }

    impl MutableProfileProvider {
        fn new(profiles: Vec<AgentProfile>) -> Self {
            Self {
                profiles: Mutex::new(profiles),
                load_call_count: AtomicUsize::new(0),
                global_call_count: AtomicUsize::new(0),
            }
        }

        fn replace_profiles(&self, profiles: Vec<AgentProfile>) {
            *self.profiles.lock().expect("profiles lock should work") = profiles;
        }

        fn load_count(&self) -> usize {
            self.load_call_count.load(Ordering::SeqCst)
        }

        fn global_count(&self) -> usize {
            self.global_call_count.load(Ordering::SeqCst)
        }
    }

    impl ProfileProvider for MutableProfileProvider {
        fn load_for_working_dir(
            &self,
            _working_dir: &Path,
        ) -> Result<Vec<AgentProfile>, ApplicationError> {
            self.load_call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .profiles
                .lock()
                .expect("profiles lock should work")
                .clone())
        }

        fn load_global(&self) -> Result<Vec<AgentProfile>, ApplicationError> {
            self.global_call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .profiles
                .lock()
                .expect("profiles lock should work")
                .clone())
        }
    }

    fn test_profile(id: &str) -> AgentProfile {
        AgentProfile {
            id: id.to_string(),
            name: id.to_string(),
            description: format!("test {id}"),
            mode: astrcode_core::AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            model_preference: None,
        }
    }

    #[test]
    fn resolve_loads_profiles_on_first_call() {
        let provider = Arc::new(StubProfileProvider::new(vec![
            test_profile("explore"),
            test_profile("plan"),
        ]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let profiles = service.resolve(&dir).expect("resolve should succeed");
        assert_eq!(profiles.len(), 2);
        assert_eq!(provider.load_count(), 1);
    }

    #[test]
    fn resolve_hits_cache_on_second_call() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let _first = service.resolve(&dir).expect("resolve should succeed");
        let second = service.resolve(&dir).expect("resolve should succeed");
        assert_eq!(second.len(), 1);
        assert_eq!(provider.load_count(), 1, "不应重复加载");
    }

    #[test]
    fn find_profile_returns_matching_profile() {
        let provider = Arc::new(StubProfileProvider::new(vec![
            test_profile("explore"),
            test_profile("plan"),
        ]));
        let service = ProfileResolutionService::new(provider);
        let dir = std::env::current_dir().expect("current_dir should be available");

        let profile = service
            .find_profile(&dir, "plan")
            .expect("find_profile should find 'plan'");
        assert_eq!(profile.id, "plan");
    }

    #[test]
    fn find_profile_returns_not_found_when_missing() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider);
        let dir = std::env::current_dir().expect("current_dir should be available");

        let err = service
            .find_profile(&dir, "nonexistent")
            .expect_err("find_profile should fail for nonexistent");
        assert!(
            matches!(err, ApplicationError::NotFound(ref msg) if msg.contains("nonexistent")),
            "should be NotFound: {err}"
        );
    }

    #[test]
    fn find_profile_returns_not_found_even_with_cache_hit() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider);
        let dir = std::env::current_dir().expect("current_dir should be available");

        // 首次访问，缓存生效
        let _ = service.resolve(&dir).expect("resolve should succeed");
        // 再次查询不存在的 profile：命中缓存但返回 NotFound
        let err = service
            .find_profile(&dir, "missing")
            .expect_err("find_profile should fail for missing");
        assert!(
            matches!(err, ApplicationError::NotFound(_)),
            "缓存命中不影响业务校验: {err}"
        );
    }

    #[test]
    fn invalidate_clears_cache_for_path() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let _ = service.resolve(&dir).expect("resolve should succeed");
        assert_eq!(provider.load_count(), 1);

        service.invalidate(&dir);
        let _ = service.resolve(&dir).expect("resolve should succeed");
        assert_eq!(provider.load_count(), 2, "失效后应重新加载");
    }

    #[test]
    fn invalidate_all_clears_everything() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let _ = service.resolve(&dir).expect("resolve should succeed");
        let _ = service
            .resolve_global()
            .expect("resolve_global should succeed");

        service.invalidate_all();

        let _ = service.resolve(&dir).expect("resolve should succeed");
        let _ = service
            .resolve_global()
            .expect("resolve_global should succeed");
        assert_eq!(provider.load_count(), 2, "全部失效后应重新加载");
        assert_eq!(provider.global_count(), 2, "全部失效后应重新加载全局");
    }

    #[test]
    fn global_cache_hit_avoids_reloading() {
        let provider = Arc::new(StubProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());

        let _first = service
            .resolve_global()
            .expect("resolve_global should succeed");
        let second = service
            .resolve_global()
            .expect("resolve_global should succeed");
        assert_eq!(second.len(), 1);
        assert_eq!(provider.global_count(), 1, "全局缓存应命中");
    }

    #[test]
    fn find_global_profile_returns_matching() {
        let provider = Arc::new(StubProfileProvider::new(vec![
            test_profile("explore"),
            test_profile("plan"),
        ]));
        let service = ProfileResolutionService::new(provider);

        let profile = service
            .find_global_profile("plan")
            .expect("find_global_profile should find 'plan'");
        assert_eq!(profile.id, "plan");
    }

    #[test]
    fn find_global_profile_returns_not_found_when_missing() {
        let provider = Arc::new(StubProfileProvider::new(vec![]));
        let service = ProfileResolutionService::new(provider);

        let err = service
            .find_global_profile("nobody")
            .expect_err("find_global_profile should fail for 'nobody'");
        assert!(
            matches!(err, ApplicationError::NotFound(ref msg) if msg.contains("nobody")),
            "should be NotFound: {err}"
        );
    }

    #[test]
    fn invalidate_global_also_clears_scoped_cache() {
        let provider = Arc::new(MutableProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let _ = service.resolve(&dir).expect("resolve should succeed");
        let _ = service
            .resolve_global()
            .expect("resolve_global should succeed");
        assert_eq!(provider.load_count(), 1);
        assert_eq!(provider.global_count(), 1);

        service.invalidate_global();
        let _ = service.resolve(&dir).expect("resolve should succeed");
        let _ = service
            .resolve_global()
            .expect("resolve_global should succeed");
        assert_eq!(provider.load_count(), 2, "全局失效也应清理 scoped cache");
        assert_eq!(provider.global_count(), 2, "全局 cache 应重新加载");
    }

    #[test]
    fn invalidate_reloads_future_requests_without_mutating_existing_snapshot() {
        let provider = Arc::new(MutableProfileProvider::new(vec![test_profile("explore")]));
        let service = ProfileResolutionService::new(provider.clone());
        let dir = std::env::current_dir().expect("current_dir should be available");

        let first = service.resolve(&dir).expect("resolve should succeed");
        assert_eq!(first[0].description, "test explore");

        provider.replace_profiles(vec![AgentProfile {
            description: "updated explore".to_string(),
            ..test_profile("explore")
        }]);
        service.invalidate(&dir);
        let second = service.resolve(&dir).expect("resolve should succeed");

        assert_eq!(first[0].description, "test explore");
        assert_eq!(second[0].description, "updated explore");
        assert_eq!(
            provider.load_count(),
            2,
            "失效后后续请求必须重新读取 provider"
        );
    }
}
