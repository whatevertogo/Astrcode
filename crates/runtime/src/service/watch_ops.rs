use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use astrcode_runtime_agent_loader::AgentWatchPath;
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};

use super::{RuntimeService, ServiceError, ServiceResult, blocking_bridge::spawn_blocking_service};
use crate::config::config_path;

pub(super) async fn run_config_watch_loop(service: Arc<RuntimeService>) -> ServiceResult<()> {
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
            _ = service.shutdown_token.cancelled() => return Ok(()),
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

                match service.reload_config_from_disk().await {
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
    let working_dir = std::env::current_dir().ok();
    let mut watch_targets = service.agent_loader().watch_paths(working_dir.as_deref());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
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
            _ = service.shutdown_token.cancelled() => return Ok(()),
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

                let next_watch_targets = service.agent_loader().watch_paths(working_dir.as_deref());
                if next_watch_targets != watch_targets {
                    let current = watch_targets
                        .iter()
                        .map(|target| (target.path.clone(), target.recursive))
                        .collect::<HashMap<_, _>>();
                    apply_agent_watch_targets(&mut watcher, &current, &next_watch_targets)?;
                    watch_targets = next_watch_targets;
                }

                match service.reload_agent_profiles_from_disk().await {
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
    let debounce = tokio::time::sleep(Duration::from_millis(300));
    tokio::pin!(debounce);
    loop {
        tokio::select! {
            _ = service.shutdown_token.cancelled() => return Ok(()),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::{Event, EventKind};

    use super::{event_targets_agent_dirs, event_targets_config};

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
}
