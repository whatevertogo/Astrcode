//! # 会话操作 (Session Operations)
//!
//! 实现 `RuntimeService` 的会话生命周期管理，包括：
//! - 创建会话（生成唯一 ID、初始化事件日志）
//! - 加载会话快照（从磁盘读取历史事件）
//! - 列出会话（带元数据）
//! - 删除会话/项目
//! - 分支会话（从现有会话创建新分支）
//! - 提交 Prompt（触发 Turn 执行）
//!
//! ## 会话 ID 规范化
//!
//! 所有公开 API 接受会话 ID 时都会通过 `normalize_session_id` 处理，
//! 去除前后空白，避免用户输入错误导致的会话查找失败。
//!
//! ## 分支会话
//!
//! 分支会话创建一个新的子会话，继承父会话的工作目录和历史事件。
//! 分支深度限制为 3 层，避免过深的分支树导致性能问题。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use astrcode_core::{
    AgentStateProjector, AstrError, DeleteProjectResult, Phase, SessionMeta, StorageEvent,
    StoredEvent, generate_session_id, phase_of_storage_event, replay_records,
};
use chrono::Utc;

use super::{
    RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent, SessionMessage,
    replay::convert_events_to_messages,
    session_state::{SessionState, SessionWriter},
    support::spawn_blocking_service,
};

impl RuntimeService {
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        let session_manager = Arc::clone(&self.session_manager);
        spawn_blocking_service("list sessions with metadata", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        let working_dir = working_dir.into();
        let session_manager = Arc::clone(&self.session_manager);
        let (session_id, working_dir, created_at, log, stored_session_start) =
            spawn_blocking_service("create session", move || {
                let working_dir = normalize_working_dir(working_dir)?;
                let session_id = generate_session_id();
                let mut log = session_manager
                    .create_event_log(&session_id, &working_dir)
                    .map_err(ServiceError::from)?;
                let created_at = Utc::now();
                let session_start = StorageEvent::SessionStart {
                    session_id: session_id.clone(),
                    timestamp: created_at,
                    working_dir: working_dir.to_string_lossy().to_string(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                };
                let stored_session_start =
                    log.append(&session_start).map_err(ServiceError::from)?;
                Ok((
                    session_id,
                    working_dir,
                    created_at,
                    log,
                    stored_session_start,
                ))
            })
            .await?;

        let phase = phase_of_storage_event(&stored_session_start.event);
        let state = Arc::new(SessionState::new(
            phase,
            Arc::new(SessionWriter::new(log)),
            AgentStateProjector::from_events(std::slice::from_ref(&stored_session_start.event)),
            replay_records(std::slice::from_ref(&stored_session_start), None),
            vec![stored_session_start.clone()],
        ));
        self.sessions.insert(session_id.clone(), state);

        let meta = SessionMeta {
            session_id,
            working_dir: working_dir.to_string_lossy().to_string(),
            display_name: display_name_from_working_dir(&working_dir),
            title: "新会话".to_string(),
            created_at,
            updated_at: created_at,
            parent_session_id: None,
            parent_storage_seq: None,
            phase: Phase::Idle,
        };

        self.emit_session_catalog_event(SessionCatalogEvent::SessionCreated {
            session_id: meta.session_id.clone(),
        });

        Ok(meta)
    }

    pub async fn load_session_snapshot(
        &self,
        session_id: &str,
    ) -> ServiceResult<(Vec<SessionMessage>, Option<String>)> {
        let session_id = normalize_session_id(session_id);
        let events = load_events(Arc::clone(&self.session_manager), &session_id).await?;
        let cursor = replay_records(&events, None)
            .last()
            .map(|record| record.event_id.clone());
        Ok((convert_events_to_messages(&events), cursor))
    }

    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let normalized = normalize_session_id(session_id);
        let _guard = self.session_load_lock.lock().await;
        self.interrupt(&normalized).await?;
        self.sessions.remove(&normalized);
        let session_manager = Arc::clone(&self.session_manager);
        let delete_session_id = normalized.clone();
        spawn_blocking_service("delete session", move || {
            session_manager
                .delete_session(&delete_session_id)
                .map_err(ServiceError::from)
        })
        .await?;
        self.emit_session_catalog_event(SessionCatalogEvent::SessionDeleted {
            session_id: normalized,
        });
        Ok(())
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        let working_dir = working_dir.to_string();
        let session_manager = Arc::clone(&self.session_manager);
        let metas = spawn_blocking_service("list project sessions", move || {
            session_manager
                .list_sessions_with_meta()
                .map_err(ServiceError::from)
        })
        .await?;
        let targets = metas
            .into_iter()
            .filter(|meta| meta.working_dir == working_dir)
            .map(|meta| meta.session_id)
            .collect::<Vec<_>>();

        for session_id in &targets {
            let _ = self.interrupt(session_id).await;
            self.sessions.remove(session_id);
        }

        let delete_working_dir = working_dir.clone();
        let session_manager = Arc::clone(&self.session_manager);
        let result = spawn_blocking_service("delete project sessions", move || {
            session_manager
                .delete_sessions_by_working_dir(&delete_working_dir)
                .map_err(ServiceError::from)
        })
        .await?;
        self.emit_session_catalog_event(SessionCatalogEvent::ProjectDeleted { working_dir });
        Ok(result)
    }

    /// 确保会话已加载到内存中，使用双重检查锁定避免重复加载。
    ///
    /// ## 为什么需要锁
    ///
    /// 多个并发请求可能同时请求同一个 session_id（如多个 SSE 客户端连接）。
    /// 如果不加锁，会导致同一个会话被从磁盘加载两次，创建两个不同的
    /// `SessionState` 和 broadcast channel，后续事件会分散到不同 channel。
    ///
    /// 使用 `session_load_lock` 保证只有一个请求执行实际的磁盘加载，
    /// 其他请求等待锁释放后直接从 `sessions` map 中获取已加载的状态。
    ///
    /// ## 双重检查的 DashMap 安全性
    ///
    /// 第一次检查（`self.sessions.get`）是 DashMap 的无锁读操作，开销极低。
    /// 第二次检查（锁内再 `get` 一次）是标准的 double-checked locking 模式。
    /// DashMap 的内部分片锁保证了读取时的可见性，无需额外的同步原语。
    pub(super) async fn ensure_session_loaded(
        &self,
        session_id: &str,
    ) -> ServiceResult<Arc<SessionState>> {
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let _guard = self.session_load_lock.lock().await;
        if let Some(existing) = self.sessions.get(session_id) {
            return Ok(existing.clone());
        }

        let session_id_owned = session_id.to_string();
        let session_manager = Arc::clone(&self.session_manager);
        let started_at = Instant::now();
        let load_result = spawn_blocking_service("load session state", move || {
            let stored: Vec<StoredEvent> = session_manager
                .replay_events(&session_id_owned)
                .map_err(ServiceError::from)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(ServiceError::from)?;
            let Some(first) = stored.first() else {
                return Err(ServiceError::NotFound(format!(
                    "session '{}' is empty",
                    session_id_owned
                )));
            };

            let working_dir = match &first.event {
                StorageEvent::SessionStart { working_dir, .. } => PathBuf::from(working_dir),
                _ => {
                    return Err(ServiceError::Internal(AstrError::Internal(format!(
                        "session '{}' is missing sessionStart",
                        session_id_owned
                    ))));
                },
            };
            let phase = stored
                .last()
                .map(|event| phase_of_storage_event(&event.event))
                .unwrap_or(Phase::Idle);
            let log = session_manager
                .open_event_log(&session_id_owned)
                .map_err(ServiceError::from)?;
            let events = stored
                .iter()
                .map(|record| record.event.clone())
                .collect::<Vec<_>>();
            // 从全部历史事件重建投影。对大session（数千事件）有计算成本，
            // 但发生在 load 路径（仅首次加载或服务重启时触发），不影响热路径。
            // TODO:未来可考虑在 checkpoint 处快照投影状态以加速加载。
            let projector = AgentStateProjector::from_events(&events);
            let recent_records = replay_records(&stored, None);
            Ok((working_dir, phase, log, projector, recent_records, stored))
        })
        .await;
        let elapsed = started_at.elapsed();
        match &load_result {
            Ok(_) => {
                self.observability.record_session_rehydrate(elapsed, true);
                if elapsed.as_millis() >= 250 {
                    log::warn!(
                        "session '{}' rehydrate took {}ms",
                        session_id,
                        elapsed.as_millis()
                    );
                }
            },
            Err(error) => {
                self.observability.record_session_rehydrate(elapsed, false);
                log::error!(
                    "failed to rehydrate session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            },
        }
        let (_working_dir, phase, log, projector, recent_records, recent_stored) = load_result?;

        let state = Arc::new(SessionState::new(
            phase,
            Arc::new(SessionWriter::new(log)),
            projector,
            recent_records,
            recent_stored,
        ));
        self.sessions.insert(session_id.to_string(), state.clone());
        Ok(state)
    }
}

/// 规范化会话 ID，去除首尾空白并剥离 `session-` 前缀。
///
/// session_id 既来自路径参数也来自前端状态；先裁掉首尾空白可以把
/// copy/paste 带来的噪音折叠成同一个 canonical key，避免 Windows
/// 文件名校验和 DashMap 命中结果出现分裂。
pub(super) fn normalize_session_id(session_id: &str) -> String {
    // session_id 既来自路径参数也来自前端状态；先裁掉首尾空白可以把
    // copy/paste 带来的噪音折叠成同一个 canonical key，避免 Windows
    // 文件名校验和 DashMap 命中结果出现分裂。
    let trimmed = session_id.trim();
    trimmed
        .strip_prefix("session-")
        .unwrap_or(trimmed)
        .to_string()
}

/// 规范化工作目录路径，验证其存在且为目录。
///
/// 若传入相对路径，会先解析为绝对路径再 canonicalize。
/// 拒绝文件路径（必须是目录），确保会话-项目关联的正确性。
pub(super) fn normalize_working_dir(working_dir: PathBuf) -> ServiceResult<PathBuf> {
    let path = if working_dir.is_absolute() {
        working_dir
    } else {
        std::env::current_dir()
            .map_err(|error| {
                ServiceError::Internal(AstrError::io("failed to get current directory", error))
            })?
            .join(working_dir)
    };

    let metadata = std::fs::metadata(&path).map_err(|error| {
        ServiceError::InvalidInput(format!(
            "workingDir '{}' is invalid: {}",
            path.display(),
            error
        ))
    })?;
    if !metadata.is_dir() {
        return Err(ServiceError::InvalidInput(format!(
            "workingDir '{}' is not a directory",
            path.display()
        )));
    }

    // canonicalize 解析符号链接并规范化路径，确保不同路径表示
    // （如 macOS 的 /tmp → /private/tmp，Windows 的大小写差异）
    // 映射到同一物理目录。这对会话-项目关联的正确性至关重要：
    // 否则同一路径的两个表示会产生两个独立的会话集。
    std::fs::canonicalize(&path)
        .map_err(|e| {
            AstrError::io(
                format!("failed to canonicalize workingDir '{}'", path.display()),
                e,
            )
        })
        .map_err(ServiceError::from)
}

/// 从工作目录路径提取显示名称。
///
/// 优先使用路径的最后一部分（文件夹名），
/// 若无法提取则回退到 "默认项目"。
pub(super) fn display_name_from_working_dir(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

/// 从磁盘加载会话的全部历史事件。
///
/// 通过 `spawn_blocking_service` 将文件 I/O 委托给阻塞线程池，
/// 避免阻塞 async 运行时。
pub(super) async fn load_events(
    session_manager: Arc<dyn astrcode_core::SessionManager>,
    session_id: &str,
) -> ServiceResult<Vec<StoredEvent>> {
    let session_id = session_id.to_string();
    spawn_blocking_service("load session events", move || {
        session_manager
            .replay_events(&session_id)
            .map_err(ServiceError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ServiceError::from)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use astrcode_core::project::project_dir_name;

    use super::*;
    use crate::test_support::{TestEnvGuard, empty_capabilities};

    #[tokio::test]
    async fn ensure_session_loaded_reuses_single_state_under_concurrency() {
        let _guard = TestEnvGuard::new();
        let service = Arc::new(RuntimeService::from_capabilities(empty_capabilities()).unwrap());
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let meta = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        service.sessions.remove(&meta.session_id);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            let session_id = meta.session_id.clone();
            handles.push(tokio::spawn(async move {
                service
                    .ensure_session_loaded(&session_id)
                    .await
                    .expect("session should load")
            }));
        }

        let states = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|result| result.expect("task should join"))
            .collect::<Vec<_>>();

        let first = Arc::as_ptr(&states[0]);
        assert!(
            states
                .iter()
                .all(|state| std::ptr::eq(Arc::as_ptr(state), first))
        );
        assert_eq!(service.sessions.len(), 1);
    }

    #[tokio::test]
    async fn create_session_persists_into_project_bucket_directory() {
        let guard = TestEnvGuard::new();
        let service = RuntimeService::from_capabilities(empty_capabilities()).unwrap();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        let meta = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");

        let projects_root = guard.home_dir().join(".astrcode").join("projects");
        assert!(
            !guard
                .home_dir()
                .join(".astrcode")
                .join("sessions")
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "new layout should avoid writing fresh sessions back into the legacy flat root"
        );

        let bucket_dir = projects_root
            .join(project_dir_name(temp_dir.path()))
            .join("sessions");
        let session_dir = bucket_dir.join(&meta.session_id);
        assert!(
            session_dir
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "session file should be nested under a per-session directory inside the project bucket"
        );
    }

    #[test]
    fn normalize_session_id_keeps_legacy_inner_prefix() {
        assert_eq!(
            normalize_session_id("session-session-2026-03-08T10-00-00-aaaaaaaa"),
            "session-2026-03-08T10-00-00-aaaaaaaa"
        );
    }

    #[test]
    fn normalize_session_id_trims_outer_whitespace_before_removing_prefix() {
        assert_eq!(normalize_session_id("session-abc "), "abc");
        assert_eq!(normalize_session_id(" session-abc"), "abc");
        assert_eq!(normalize_session_id(" abc "), "abc");
    }

    #[test]
    fn normalize_working_dir_rejects_file_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let file = temp_dir.path().join("file.txt");
        std::fs::write(&file, "demo").expect("file should be created");

        let err =
            normalize_working_dir(file).expect_err("file paths should not be accepted as workdir");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("is not a directory"));
    }

    #[test]
    fn normalize_working_dir_rejects_missing_paths() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let missing = temp_dir.path().join("missing");

        let err = normalize_working_dir(missing).expect_err("missing workdir should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("is invalid"));
    }

    #[test]
    fn display_name_from_working_dir_uses_default_for_root() {
        #[cfg(windows)]
        let root = Path::new(r"C:\");
        #[cfg(not(windows))]
        let root = Path::new("/");

        assert_eq!(display_name_from_working_dir(root), "默认项目");
    }

    #[test]
    fn display_name_from_working_dir_ignores_trailing_separator() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let rendered = format!("{}{}", temp_dir.path().display(), std::path::MAIN_SEPARATOR);

        assert_eq!(
            display_name_from_working_dir(Path::new(&rendered)),
            temp_dir
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .expect("tempdir name should be utf-8")
        );
    }
}
