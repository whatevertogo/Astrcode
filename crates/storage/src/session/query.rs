use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use astrcode_core::store::StoreError;
use astrcode_core::{
    phase_of_storage_event, DeleteProjectResult, Phase, SessionMeta, StorageEvent, StoredEventLine,
};
use chrono::{DateTime, Utc};

use crate::{internal_io_error, AstrError, Result};

use super::event_log::EventLog;
use super::paths::{canonical_session_id, is_valid_session_id, sessions_dir, validated_session_id};

impl EventLog {
    pub fn list_sessions() -> Result<Vec<String>> {
        let dir = sessions_dir()?;
        Self::list_sessions_from_path(&dir)
    }

    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>> {
        let dir = sessions_dir()?;
        Self::list_sessions_with_meta_from_path(&dir)
    }

    pub fn delete_session(session_id: &str) -> Result<()> {
        let dir = sessions_dir()?;
        Self::delete_session_from_path(&dir, session_id)
    }

    pub fn delete_sessions_by_working_dir(working_dir: &str) -> Result<DeleteProjectResult> {
        let dir = sessions_dir()?;
        Self::delete_sessions_by_working_dir_from_path(&dir, working_dir)
    }

    pub(crate) fn list_sessions_from_path(dir: &Path) -> Result<Vec<String>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in
            fs::read_dir(dir).map_err(|e| AstrError::io("failed to read sessions directory", e))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name
                .strip_prefix("session-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            {
                if is_valid_session_id(id) {
                    ids.push(id.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub(crate) fn list_sessions_with_meta_from_path(dir: &Path) -> Result<Vec<SessionMeta>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut metas = Vec::new();
        for entry in
            fs::read_dir(dir).map_err(|e| AstrError::io("failed to read sessions directory", e))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name
                .strip_prefix("session-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };

            if !is_valid_session_id(id) {
                continue;
            }

            let canonical_id = canonical_session_id(id).to_string();
            let path = entry.path();
            let (created_at, working_dir, title) = match Self::read_session_head_meta(&path) {
                Ok(meta) => meta,
                Err(error) => {
                    log::warn!(
                        "skipping unreadable session file '{}': {}",
                        path.display(),
                        error
                    );
                    continue;
                }
            };
            let updated_at = Self::read_last_timestamp(&path).unwrap_or(created_at);
            let phase = Self::read_last_phase(&path).unwrap_or(Phase::Idle);
            metas.push(SessionMeta {
                session_id: canonical_id,
                working_dir: working_dir.clone(),
                display_name: session_display_name(&working_dir),
                title,
                created_at,
                updated_at,
                phase,
            });
        }

        metas.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });

        Ok(metas)
    }

    pub(crate) fn delete_session_from_path(dir: &Path, session_id: &str) -> Result<()> {
        let canonical_id = validated_session_id(session_id)?;
        let canonical = dir.join(format!("session-{canonical_id}.jsonl"));
        let legacy = dir.join(format!("session-{session_id}.jsonl"));
        let target = if canonical.exists() {
            canonical
        } else if legacy != canonical && legacy.exists() {
            legacy
        } else {
            return Err(StoreError::SessionNotFound(canonical.display().to_string()));
        };

        fs::remove_file(&target).map_err(|e| {
            AstrError::io(
                format!("failed to delete session file: {}", target.display()),
                e,
            )
        })?;
        Ok(())
    }

    pub(crate) fn delete_sessions_by_working_dir_from_path(
        dir: &Path,
        working_dir: &str,
    ) -> Result<DeleteProjectResult> {
        let metas = Self::list_sessions_with_meta_from_path(dir)?;
        let mut success_count = 0usize;
        let mut failed_session_ids = Vec::new();

        for meta in metas.into_iter().filter(|m| m.working_dir == working_dir) {
            match Self::delete_session_from_path(dir, &meta.session_id) {
                Ok(_) => success_count += 1,
                Err(_) => failed_session_ids.push(meta.session_id),
            }
        }

        Ok(DeleteProjectResult {
            success_count,
            failed_session_ids,
        })
    }

    fn read_session_head_meta(path: &Path) -> Result<(DateTime<Utc>, String, String)> {
        let file = File::open(path).map_err(|e| {
            AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        let reader = BufReader::new(file);

        let mut created_at = None;
        let mut working_dir = None;
        let mut title = None;

        for (i, line) in reader.lines().enumerate() {
            let line =
                line.map_err(|e| AstrError::io("failed to read line from session file", e))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let event = serde_json::from_str::<StoredEventLine>(trimmed)
                .map_err(|e| {
                    AstrError::parse(
                        format!(
                            "failed to parse head event at {}:{}: {}",
                            path.display(),
                            i + 1,
                            trimmed
                        ),
                        e,
                    )
                })?
                .into_stored((i + 1) as u64)
                .event;

            match event {
                StorageEvent::SessionStart {
                    timestamp,
                    working_dir: wd,
                    ..
                } => {
                    if created_at.is_none() {
                        created_at = Some(timestamp);
                        working_dir = Some(wd);
                    }
                }
                StorageEvent::UserMessage { content, .. } if title.is_none() => {
                    title = Some(title_from_user_message(&content));
                }
                _ => {}
            }

            if created_at.is_some() && title.is_some() {
                break;
            }
        }

        let created_at = created_at.ok_or_else(|| {
            internal_io_error(format!(
                "session file missing sessionStart: {}",
                path.display()
            ))
        })?;
        let working_dir = working_dir.unwrap_or_default();
        let title = title.unwrap_or_else(|| "新会话".to_string());
        Ok((created_at, working_dir, title))
    }

    fn read_last_timestamp(path: &Path) -> Result<DateTime<Utc>> {
        Self::read_tail_value(path, timestamp_of_event)?.ok_or_else(|| {
            internal_io_error(format!(
                "unable to resolve tail timestamp from session file: {}",
                path.display()
            ))
        })
    }

    fn read_last_phase(path: &Path) -> Result<Phase> {
        Ok(
            Self::read_tail_value(path, |event| Some(phase_of_event(event)))?
                .unwrap_or(Phase::Idle),
        )
    }

    /// 从会话文件尾部扫描，查找满足 mapper 条件的最后一个值。
    ///
    /// ## 算法（指数窗口扫描）
    ///
    /// 从文件末尾向前搜索，使用指数增长的读取窗口：
    /// 1. 初始窗口 = 4KB，从 `len - window` 位置开始读取
    /// 2. 如果读取位置不在文件开头，跳过第一行（可能是不完整的行）
    /// 3. 如果跳过第一行后没有任何内容，窗口翻倍重试
    /// 4. 对窗口内的行**从后向前**遍历，找到第一个匹配的值立即返回
    /// 5. 如果窗口内没找到匹配值，翻倍窗口继续扫描更早的内容
    ///
    /// 这个策略避免了对大文件的全量扫描——对于只需要最后一个时间戳或阶段
    /// 的场景，通常在第一次 4KB 读取中就能命中。
    fn read_tail_value<T, F>(path: &Path, mut mapper: F) -> Result<Option<T>>
    where
        F: FnMut(&StorageEvent) -> Option<T>,
    {
        let file = File::open(path).map_err(|e| {
            AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        let mut reader = BufReader::new(file);
        let len = reader
            .get_ref()
            .metadata()
            .map_err(|e| {
                AstrError::io(
                    format!("failed to stat session file: {}", path.display()),
                    e,
                )
            })?
            .len();

        if len == 0 {
            return Err(internal_io_error(format!(
                "empty session file: {}",
                path.display()
            )));
        }

        let mut window: u64 = 4096;
        loop {
            let start = len.saturating_sub(window);
            reader.seek(SeekFrom::Start(start))?;

            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;

            let slice = if start > 0 {
                if let Some(pos) = bytes.iter().position(|b| *b == b'\n') {
                    &bytes[pos + 1..]
                } else if start == 0 || window >= len {
                    bytes.as_slice()
                } else {
                    window = (window * 2).min(len);
                    continue;
                }
            } else {
                bytes.as_slice()
            };

            let text = String::from_utf8_lossy(slice);
            for line in text.lines().rev() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let event = serde_json::from_str::<StoredEventLine>(trimmed)
                    .map_err(|e| {
                        AstrError::parse(
                            format!(
                                "failed to parse tail event at {}: {}",
                                path.display(),
                                trimmed
                            ),
                            e,
                        )
                    })?
                    .into_stored(0)
                    .event;
                if let Some(value) = mapper(&event) {
                    return Ok(Some(value));
                }
            }

            if start == 0 || window >= len {
                break;
            }
            window = (window * 2).min(len);
        }

        Ok(None)
    }
}

fn timestamp_of_event(event: &StorageEvent) -> Option<DateTime<Utc>> {
    match event {
        StorageEvent::SessionStart { timestamp, .. } => Some(*timestamp),
        StorageEvent::UserMessage { timestamp, .. } => Some(*timestamp),
        StorageEvent::AssistantFinal { timestamp, .. } => timestamp.as_ref().cloned(),
        StorageEvent::TurnDone { timestamp, .. } => Some(*timestamp),
        StorageEvent::Error { timestamp, .. } => timestamp.as_ref().cloned(),
        _ => None,
    }
}

/// 适配器：将 `phase_of_storage_event()`（返回 `Phase`）包装为 `Option<Phase>`，
/// 以便作为 `read_tail_value()` 的 mapper 闭包使用。
fn phase_of_event(event: &StorageEvent) -> Phase {
    phase_of_storage_event(event)
}

fn session_display_name(working_dir: &str) -> String {
    let normalized = working_dir.trim_end_matches(['/', '\\']);
    normalized
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

fn title_from_user_message(content: &str) -> String {
    let title: String = content.chars().take(20).collect();
    let title = title.trim();
    if title.is_empty() {
        "新会话".to_string()
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::TimeZone;

    use super::*;
    use astrcode_core::StoredEvent;

    #[test]
    fn read_last_timestamp_uses_error_event_timestamp() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("session-test.jsonl");
        let created_at = Utc
            .with_ymd_and_hms(2026, 3, 18, 8, 0, 0)
            .single()
            .expect("timestamp should be valid");
        let failed_at = Utc
            .with_ymd_and_hms(2026, 3, 18, 8, 5, 0)
            .single()
            .expect("timestamp should be valid");
        let lines = [
            serde_json::to_string(&StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SessionStart {
                    session_id: "session-1".to_string(),
                    timestamp: created_at,
                    working_dir: "/tmp/project".to_string(),
                },
            })
            .expect("session start should serialize"),
            serde_json::to_string(&StoredEvent {
                storage_seq: 2,
                event: StorageEvent::Error {
                    turn_id: Some("turn-1".to_string()),
                    message: "boom".to_string(),
                    timestamp: Some(failed_at),
                },
            })
            .expect("error event should serialize"),
        ];
        fs::write(&path, format!("{}\n{}\n", lines[0], lines[1])).expect("log should be written");

        let updated_at = EventLog::read_last_timestamp(&path).expect("timestamp should resolve");

        assert_eq!(updated_at, failed_at);
    }

    #[test]
    fn read_last_phase_reads_tail_event_without_loading_full_log() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("session-tail.jsonl");
        let created_at = Utc
            .with_ymd_and_hms(2026, 3, 18, 9, 0, 0)
            .single()
            .expect("timestamp should be valid");
        let lines = [
            serde_json::to_string(&StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SessionStart {
                    session_id: "session-2".to_string(),
                    timestamp: created_at,
                    working_dir: "/tmp/project".to_string(),
                },
            })
            .expect("session start should serialize"),
            serde_json::to_string(&StoredEvent {
                storage_seq: 2,
                event: StorageEvent::ToolCall {
                    turn_id: Some("turn-2".to_string()),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "grep".to_string(),
                    args: serde_json::json!({"pattern":"TODO"}),
                },
            })
            .expect("tool call should serialize"),
        ];
        fs::write(&path, format!("{}\n{}\n", lines[0], lines[1])).expect("log should be written");

        let phase = EventLog::read_last_phase(&path).expect("phase should resolve");

        assert_eq!(phase, Phase::CallingTool);
    }
}
