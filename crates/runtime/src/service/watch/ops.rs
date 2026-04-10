//! 文件系统监视操作：配置热加载与 Agent Profile 热加载
//!
//! 本模块负责监听磁盘文件变更并在检测到变化时触发热加载，包含两个独立的监视循环：
//!
//! - **配置热加载**（`run_config_watch_loop`）：监听全局配置文件（如 `config.json`），
//!   当文件内容发生变化时自动重新加载，使配置变更即时生效而无需重启服务。
//!
//! - **Agent Profile 热加载**（`run_agent_watch_loop`）：监听所有已知会话工作目录下 的
//!   `.astrcode/agents/` 目录，当 Agent 定义文件变更时自动重新加载。
//!   支持动态增删监听目标——当会话工作目录集合发生变化时，会自动调整监视范围。
//!
//! 两个循环都使用 `notify` crate 的 `RecommendedWatcher`（平台原生文件系统通知），
//! 并通过 300ms 防抖机制合并短时间内的大量文件变更事件，避免重复加载。

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use astrcode_runtime_agent_loader::AgentWatchPath;
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    config::config_path,
    service::{
        RuntimeService, ServiceError, ServiceResult, blocking_bridge::spawn_blocking_service,
    },
};

pub(super) async fn run_config_watch_loop(service: Arc<RuntimeService>) -> ServiceResult<()> {
    let shutdown = service.lifecycle().shutdown_signal();
    let watched_config_path = spawn_blocking_service("resolve config watch path", || {
        config_path().map_err(ServiceError::from)
    })
    .await?;
    let watch_dir = watched_config_path
        .parent()
        .ok_or_else(|| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                "config path '{}' has no parent directory",
                watched_config_path.display()
            )))
        })?
        .to_path_buf();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            // 故意忽略：watcher 关闭后 channel 发送失败是正常的
            let _ = tx.send(result);
        },
        NotifyConfig::default(),
    )
    .map_err(|error| {
        ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
            "failed to create config watcher: {error}"
        )))
    })?;

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|error| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                "failed to watch config directory '{}': {error}",
                watch_dir.display()
            )))
        })?;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            maybe_event = rx.recv() => {
                let Some(result) = maybe_event else {
                    return Ok(());
                };

                match result {
                    Ok(event) => {
                        if !event_targets_config(&event, &watched_config_path) {
                            continue;
                        }
                    }
                    Err(error) => {
                        log::warn!("config watcher delivered an error: {}", error);
                        continue;
                    }
                }

                drain_watch_events_with_debounce(&service, &mut rx, "config").await?;

                match service.config().reload_config_from_disk().await {
                    Ok(config) => {
                        log::info!(
                            "reloaded config from disk: active_profile='{}', active_model='{}'",
                            config.active_profile,
                            config.active_model
                        );
                    }
                    Err(error) => {
                        log::warn!("failed to hot-reload config from disk: {}", error);
                    }
                }
            }
        }
    }
}

pub(super) async fn run_agent_watch_loop(service: Arc<RuntimeService>) -> ServiceResult<()> {
    let shutdown = service.lifecycle().shutdown_signal();
    let mut watch_targets = resolve_agent_watch_targets(&service).await?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            // 故意忽略：watcher 关闭后 channel 发送失败是正常的
            let _ = tx.send(result);
        },
        NotifyConfig::default(),
    )
    .map_err(|error| {
        ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
            "failed to create agent watcher: {error}"
        )))
    })?;

    apply_agent_watch_targets(&mut watcher, &HashMap::new(), &watch_targets)?;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            maybe_event = rx.recv() => {
                let Some(result) = maybe_event else {
                    return Ok(());
                };

                match result {
                    Ok(event) => {
                        if !event_targets_agent_dirs(&event, &watch_targets) {
                            continue;
                        }
                    }
                    Err(error) => {
                        log::warn!("agent watcher delivered an error: {}", error);
                        continue;
                    }
                }

                drain_watch_events_with_debounce(&service, &mut rx, "agent").await?;

                let next_watch_targets = resolve_agent_watch_targets(&service).await?;
                if next_watch_targets != watch_targets {
                    let current = watch_targets
                        .iter()
                        .map(|target| (target.path.clone(), target.recursive))
                        .collect::<HashMap<_, _>>();
                    apply_agent_watch_targets(&mut watcher, &current, &next_watch_targets)?;
                    watch_targets = next_watch_targets;
                }

                match service.config().reload_agent_profiles_from_disk().await {
                    Ok(registry) => {
                        log::info!(
                            "reloaded agent profiles from disk: {} agents",
                            registry.list().len()
                        );
                    }
                    Err(error) => {
                        log::warn!("failed to hot-reload agent profiles from disk: {}", error);
                    }
                }
            }
        }
    }
}

fn event_targets_config(event: &Event, config_path: &std::path::Path) -> bool {
    let Some(config_file_name) = config_path.file_name() else {
        return false;
    };

    event.paths.iter().any(|path| {
        path == config_path
            || path
                .file_name()
                .is_some_and(|file_name| file_name == config_file_name)
    })
}

fn event_targets_agent_dirs(event: &Event, watch_targets: &[AgentWatchPath]) -> bool {
    event.paths.iter().any(|path| {
        watch_targets
            .iter()
            .any(|watch_target| path == &watch_target.path || path.starts_with(&watch_target.path))
    })
}

async fn drain_watch_events_with_debounce(
    service: &RuntimeService,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<notify::Result<Event>>,
    watcher_name: &str,
) -> ServiceResult<()> {
    let shutdown = service.shutdown_token.clone();
    let debounce = tokio::time::sleep(Duration::from_millis(300));
    tokio::pin!(debounce);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            _ = &mut debounce => return Ok(()),
            maybe_next = rx.recv() => {
                let Some(next) = maybe_next else {
                    return Ok(());
                };
                if let Err(error) = next {
                    log::warn!("{watcher_name} watcher delivered an error: {error}");
                }
            }
        }
    }
}

fn apply_agent_watch_targets(
    watcher: &mut RecommendedWatcher,
    current: &HashMap<PathBuf, bool>,
    next: &[AgentWatchPath],
) -> ServiceResult<()> {
    let next_map = next
        .iter()
        .map(|target| (target.path.clone(), target.recursive))
        .collect::<HashMap<_, _>>();

    for (path, recursive) in current {
        if next_map.get(path) == Some(recursive) {
            continue;
        }
        watcher.unwatch(path).map_err(|error| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                "failed to stop watching agent path '{}': {error}",
                path.display()
            )))
        })?;
    }

    for target in next {
        if current.get(&target.path) == Some(&target.recursive) {
            continue;
        }
        watcher
            .watch(
                &target.path,
                if target.recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                },
            )
            .map_err(|error| {
                ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                    "failed to watch agent path '{}'{}: {error}",
                    target.path.display(),
                    if target.recursive { " recursively" } else { "" }
                )))
            })?;
    }

    Ok(())
}

async fn resolve_agent_watch_targets(
    service: &RuntimeService,
) -> ServiceResult<Vec<AgentWatchPath>> {
    let session_manager = Arc::clone(&service.session_manager);
    let working_dirs = spawn_blocking_service("list agent working dirs", move || {
        session_manager
            .list_sessions_with_meta()
            .map(|metas| {
                let mut working_dirs = metas
                    .into_iter()
                    .map(|meta| PathBuf::from(meta.working_dir))
                    .collect::<Vec<_>>();
                working_dirs.sort();
                working_dirs.dedup();
                working_dirs
            })
            .map_err(ServiceError::from)
    })
    .await?;
    let working_dir_refs = working_dirs
        .iter()
        .map(|path| path.as_path())
        .collect::<Vec<_>>();
    Ok(service
        .agent_loader()
        .watch_paths_for_working_dirs(working_dir_refs))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use notify::{Event, EventKind};

    use super::{event_targets_agent_dirs, event_targets_config, resolve_agent_watch_targets};
    use crate::test_support::{TestEnvGuard, empty_capabilities};

    #[test]
    fn event_targets_config_matches_exact_path_and_same_filename() {
        // 使用跨平台路径格式，避免 Windows 特有路径在 Linux CI 上失败
        let config_path = PathBuf::from("/home/test/.astrcode/config.json");
        let exact = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(config_path.clone());
        let sibling = Event::new(EventKind::Create(notify::event::CreateKind::File))
            .add_path(PathBuf::from("/opt/shadow/config.json"));
        let other = Event::new(EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::Both,
        )))
        .add_path(PathBuf::from("/home/test/.astrcode/settings.toml"));

        assert!(event_targets_config(&exact, &config_path));
        assert!(event_targets_config(&sibling, &config_path));
        assert!(!event_targets_config(&other, &config_path));
    }

    #[test]
    fn event_targets_agent_dirs_matches_watched_roots_and_descendants() {
        // 使用跨平台路径格式，避免 Windows 特有路径在 Linux CI 上失败
        let watch_targets = vec![
            astrcode_runtime_agent_loader::AgentWatchPath {
                path: PathBuf::from("/home/test/.astrcode/agents"),
                recursive: false,
            },
            astrcode_runtime_agent_loader::AgentWatchPath {
                path: PathBuf::from("/opt/repo/.astrcode/agents"),
                recursive: true,
            },
        ];
        let direct = Event::new(EventKind::Create(notify::event::CreateKind::File))
            .add_path(PathBuf::from("/home/test/.astrcode/agents/review.md"));
        let descendant = Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(PathBuf::from("/opt/repo/.astrcode/agents/nested/plan.md"));
        let unrelated = Event::new(EventKind::Remove(notify::event::RemoveKind::File))
            .add_path(PathBuf::from("/opt/repo/README.md"));

        assert!(event_targets_agent_dirs(&direct, &watch_targets));
        assert!(event_targets_agent_dirs(&descendant, &watch_targets));
        assert!(!event_targets_agent_dirs(&unrelated, &watch_targets));
    }

    #[tokio::test]
    async fn resolve_agent_watch_targets_uses_session_working_dirs_instead_of_process_cwd() {
        let _guard = TestEnvGuard::new();
        let service = Arc::new(
            super::super::RuntimeService::from_capabilities(empty_capabilities())
                .expect("service should initialize"),
        );
        let workspace = tempfile::tempdir().expect("tempdir should be created");
        let repo = workspace.path().join("repo");
        let nested = repo.join("apps").join("desktop");
        std::fs::create_dir_all(&nested).expect("nested dir should exist");
        std::fs::create_dir_all(repo.join(".astrcode").join("agents"))
            .expect("repo agents dir should exist");

        let _session = service
            .sessions()
            .create(&nested)
            .await
            .expect("session should be created");

        let watch_targets = resolve_agent_watch_targets(&service)
            .await
            .expect("watch targets should resolve");

        // Windows 上路径可能带 \\?\ UNC 前缀，用后缀匹配避免前缀不一致
        let agents_suffix = std::path::MAIN_SEPARATOR_STR.to_string()
            + ".astrcode"
            + std::path::MAIN_SEPARATOR_STR
            + "agents";
        let agents_suffix_alt = "/.astrcode/agents";
        let repo_agents = watch_targets.iter().find(|target| {
            (target.path.to_string_lossy().ends_with(&agents_suffix)
                || target.path.to_string_lossy().ends_with(agents_suffix_alt))
                && target.recursive
        });
        assert!(
            repo_agents.is_some(),
            "watch targets should contain repo .astrcode/agents with recursive=true; \
             targets={watch_targets:?}"
        );
    }
}
