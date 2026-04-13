use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use astrcode_adapter_agents::{AgentProfileLoader, AgentWatchPath};
use astrcode_application::{App, ApplicationError, WatchPort, WatchService, WatchSource};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::broadcast;

pub(crate) fn build_watch_service(
    loader: AgentProfileLoader,
) -> Result<Arc<WatchService>, ApplicationError> {
    Ok(Arc::new(WatchService::new(Arc::new(
        FileSystemWatchPort::new(loader),
    ))))
}

pub(crate) async fn bootstrap_profile_watch_runtime(
    app: Arc<App>,
    watch_service: Arc<WatchService>,
) -> Result<ProfileWatchRuntime, ApplicationError> {
    let mut sources = desired_agent_watch_sources(&app).await?;
    watch_service.start_watch(sources.iter().cloned().collect())?;

    let mut catalog_rx = app.subscribe_catalog();
    let watch_app = Arc::downgrade(&app);
    let watch_service_for_catalog = Arc::clone(&watch_service);
    let catalog_task = tokio::spawn(async move {
        loop {
            if catalog_rx.recv().await.is_err() {
                break;
            }
            let Some(watch_app) = watch_app.upgrade() else {
                break;
            };
            let next_sources = match desired_agent_watch_sources(&watch_app).await {
                Ok(next) => next,
                Err(error) => {
                    log::warn!("failed to recompute agent watch sources: {error}");
                    continue;
                },
            };

            for source in next_sources.difference(&sources) {
                if let Err(error) = watch_service_for_catalog.add_source(source.clone()) {
                    log::warn!("failed to add watch source '{source:?}': {error}");
                }
            }
            for source in sources.difference(&next_sources) {
                if let Err(error) = watch_service_for_catalog.remove_source(source) {
                    log::warn!("failed to remove watch source '{source:?}': {error}");
                }
            }
            sources = next_sources;
        }
    });

    let profiles = Arc::clone(app.profiles());
    let mut watch_rx = watch_service.subscribe();
    let event_task = tokio::spawn(async move {
        loop {
            let event = match watch_rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!(
                        "agent watch receiver lagged by {} events; falling back to invalidate_all",
                        skipped
                    );
                    profiles.invalidate_all();
                    continue;
                },
                Err(broadcast::error::RecvError::Closed) => break,
            };
            match event.source {
                WatchSource::GlobalAgentDefinitions => profiles.invalidate_global(),
                WatchSource::AgentDefinitions { working_dir } => {
                    profiles.invalidate(Path::new(&working_dir));
                },
                _ => {},
            }
        }
    });

    Ok(ProfileWatchRuntime {
        watch_service,
        catalog_task,
        event_task,
    })
}

pub(crate) struct ProfileWatchRuntime {
    watch_service: Arc<WatchService>,
    catalog_task: tokio::task::JoinHandle<()>,
    event_task: tokio::task::JoinHandle<()>,
}

impl Drop for ProfileWatchRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.watch_service.stop_all() {
            log::warn!("failed to stop profile watch service during drop: {error}");
        }
        self.catalog_task.abort();
        self.event_task.abort();
    }
}

async fn desired_agent_watch_sources(
    app: &Arc<App>,
) -> Result<HashSet<WatchSource>, ApplicationError> {
    let sessions = app.list_sessions().await?;
    let mut sources = HashSet::from([WatchSource::GlobalAgentDefinitions]);
    for session in sessions {
        sources.insert(WatchSource::AgentDefinitions {
            working_dir: session.working_dir,
        });
    }
    Ok(sources)
}

#[derive(Debug, Default)]
struct WatchRegistry {
    source_targets: HashMap<WatchSource, Vec<AgentWatchPath>>,
    watched_targets: HashMap<(PathBuf, bool), usize>,
}

struct FileSystemWatchPort {
    loader: AgentProfileLoader,
    watcher: Mutex<Option<RecommendedWatcher>>,
    registry: Arc<Mutex<WatchRegistry>>,
}

impl FileSystemWatchPort {
    fn new(loader: AgentProfileLoader) -> Self {
        Self {
            loader,
            watcher: Mutex::new(None),
            registry: Arc::new(Mutex::new(WatchRegistry::default())),
        }
    }

    fn ensure_watcher(
        &self,
        tx: broadcast::Sender<astrcode_application::WatchEvent>,
    ) -> Result<(), ApplicationError> {
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|_| ApplicationError::Internal("watcher lock poisoned".to_string()))?;
        if watcher_guard.is_some() {
            return Ok(());
        }
        let registry = Arc::clone(&self.registry);
        let watcher = RecommendedWatcher::new(
            move |result: std::result::Result<Event, notify::Error>| match result {
                Ok(event) => dispatch_watch_event(&registry, &tx, event),
                Err(error) => log::warn!("agent watch received notify error: {}", error),
            },
            NotifyConfig::default(),
        )
        .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        *watcher_guard = Some(watcher);
        Ok(())
    }

    fn resolve_targets(&self, source: &WatchSource) -> Vec<AgentWatchPath> {
        match source {
            WatchSource::GlobalAgentDefinitions => self.loader.watch_paths(None),
            WatchSource::AgentDefinitions { working_dir } => {
                self.loader.watch_paths(Some(Path::new(working_dir)))
            },
            _ => Vec::new(),
        }
    }

    fn add_source_inner(&self, source: WatchSource) -> Result<(), ApplicationError> {
        let targets = self.resolve_targets(&source);
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|_| ApplicationError::Internal("watcher lock poisoned".to_string()))?;
        let Some(watcher) = watcher_guard.as_mut() else {
            return Err(ApplicationError::Internal(
                "watcher must be initialized before adding sources".to_string(),
            ));
        };
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ApplicationError::Internal("watch registry lock poisoned".to_string()))?;
        if registry.source_targets.contains_key(&source) {
            return Ok(());
        }
        for target in &targets {
            let key = (target.path.clone(), target.recursive);
            let entry = registry.watched_targets.entry(key).or_insert(0);
            if *entry == 0 {
                watcher
                    .watch(
                        &target.path,
                        if target.recursive {
                            RecursiveMode::Recursive
                        } else {
                            RecursiveMode::NonRecursive
                        },
                    )
                    .map_err(|error| ApplicationError::Internal(error.to_string()))?;
            }
            *entry += 1;
        }
        registry.source_targets.insert(source, targets);
        Ok(())
    }

    fn remove_source_inner(&self, source: &WatchSource) -> Result<(), ApplicationError> {
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|_| ApplicationError::Internal("watcher lock poisoned".to_string()))?;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ApplicationError::Internal("watch registry lock poisoned".to_string()))?;
        let Some(targets) = registry.source_targets.remove(source) else {
            return Ok(());
        };
        let Some(watcher) = watcher_guard.as_mut() else {
            return Ok(());
        };
        for target in targets {
            let key = (target.path.clone(), target.recursive);
            let should_unwatch = if let Some(count) = registry.watched_targets.get_mut(&key) {
                *count = count.saturating_sub(1);
                *count == 0
            } else {
                false
            };
            if should_unwatch {
                registry.watched_targets.remove(&key);
                watcher
                    .unwatch(&target.path)
                    .map_err(|error| ApplicationError::Internal(error.to_string()))?;
            }
        }
        Ok(())
    }
}

impl WatchPort for FileSystemWatchPort {
    fn start_watch(
        &self,
        sources: Vec<WatchSource>,
        tx: broadcast::Sender<astrcode_application::WatchEvent>,
    ) -> Result<(), ApplicationError> {
        self.ensure_watcher(tx)?;
        for source in sources {
            self.add_source_inner(source)?;
        }
        Ok(())
    }

    fn stop_all(&self) -> Result<(), ApplicationError> {
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|_| ApplicationError::Internal("watcher lock poisoned".to_string()))?;
        *watcher_guard = None;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ApplicationError::Internal("watch registry lock poisoned".to_string()))?;
        registry.source_targets.clear();
        registry.watched_targets.clear();
        Ok(())
    }

    fn add_source(&self, source: WatchSource) -> Result<(), ApplicationError> {
        self.add_source_inner(source)
    }

    fn remove_source(&self, source: &WatchSource) -> Result<(), ApplicationError> {
        self.remove_source_inner(source)
    }
}

fn dispatch_watch_event(
    registry: &Arc<Mutex<WatchRegistry>>,
    tx: &broadcast::Sender<astrcode_application::WatchEvent>,
    event: Event,
) {
    let registry = match registry.lock() {
        Ok(registry) => registry,
        Err(_) => {
            log::warn!("watch registry lock poisoned while dispatching event");
            return;
        },
    };
    for (source, targets) in &registry.source_targets {
        let affected_paths = event
            .paths
            .iter()
            .filter(|path| {
                targets
                    .iter()
                    .any(|target| path_matches_target(path, target))
            })
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();
        if affected_paths.is_empty() {
            continue;
        }
        let _ = tx.send(astrcode_application::WatchEvent {
            source: source.clone(),
            affected_paths,
        });
    }
}

fn path_matches_target(path: &Path, target: &AgentWatchPath) -> bool {
    if target.recursive {
        return path.starts_with(&target.path);
    }
    path == target.path
        || path
            .parent()
            .is_some_and(|parent| parent == target.path.as_path())
}
