//! # 服务端文件日志
//!
//! 为 astrcode-server 增加持久化文件日志。warn 及以上级别同时写入：
//! - **归档文件**：`~/.astrcode/logs/server-YYYY-MM-DD-HHMMSS-mmm-pid<PID>.log`，每次启动新建
//! - **固定入口**：`~/.astrcode/logs/server-current.log`，每次启动截断重建
//! - **快速跳转**：`~/.astrcode/logs/server-latest.txt`，一行文字指向当前归档文件名
//!
//! ## 目录结构
//!
//! ```text
//! ~/.astrcode/logs/
//! ├── server-current.log              ← 固定入口，永远指向"这次启动"
//! ├── server-latest.txt               ← 当前归档文件名，方便脚本跳转
//! ├── server-2026-04-12-153012-417-pid12345.log
//! └── ...
//! ```
//!
//! ## 设计决策
//!
//! - archive 是持久化底线，必需；current / latest 是增强，best-effort
//! - 文件写失败用 eprintln! 暴露，不用 log::error!（防递归）
//! - 归档保留最近 10 个，按文件名字典序排序（含时间戳，字典序 = 时间序）

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    sync::Mutex,
};

use anyhow::Result;
use astrcode_core::project::astrcode_dir;
use chrono::Local;

/// 归档文件名前缀，用于匹配和清理历史归档
const LOG_FILE_PREFIX: &str = "server-";
/// 保留的最大归档文件数量（不含 server-current.log 和 server-latest.txt）
const MAX_ARCHIVE_FILES: usize = 10;
/// 文件通道的日志级别阈值：warn 及以上
const FILE_LOG_LEVEL: log::Level = log::Level::Warn;
/// 固定入口文件名
const CURRENT_LOG_NAME: &str = "server-current.log";
/// 归档文件名指针
const LATEST_LINK_NAME: &str = "server-latest.txt";

/// 服务端三通道日志器：stderr + 归档文件 + 固定入口文件。
///
/// archive 是持久化底线，必需。current 是增强体验，best-effort。
struct ServerLogger {
    /// env_logger 实例：处理 stderr 输出（含其自身的过滤逻辑）
    stderr: env_logger::Logger,
    /// 归档文件：唯一命名，保留历史
    archive: Mutex<BufWriter<File>>,
    /// 固定入口文件：每次启动截断，永远指向"当前"。best-effort。
    current: Option<Mutex<BufWriter<File>>>,
}

impl log::Log for ServerLogger {
    /// 双通道联合判断：任一通道需要即放行。
    /// release 下 stderr 可能只收 error+，但文件通道仍需独立收 warn+。
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.stderr.enabled(metadata) || metadata.level() <= FILE_LOG_LEVEL
    }

    fn log(&self, record: &log::Record) {
        // 转发给 stderr（env_logger 内部自行过滤）
        self.stderr.log(record);

        // 文件通道：独立判断 warn+，不受 stderr 过滤影响
        if record.level() <= FILE_LOG_LEVEL {
            // 格式化一次，两个文件共用同一行，避免重复计算
            let line = format!(
                "[{} {:5} {}] {}\n",
                Local::now().format("%Y-%m-%dT%H:%M:%S%.3f"),
                record.level(),
                record.target(),
                record.args()
            );

            if let Ok(mut writer) = self.archive.lock() {
                write_line(&mut writer, "archive", &line);
            }

            if let Some(current) = &self.current {
                if let Ok(mut writer) = current.lock() {
                    write_line(&mut writer, "current", &line);
                }
            }
        }
    }

    fn flush(&self) {
        self.stderr.flush();
        if let Ok(mut writer) = self.archive.lock() {
            if let Err(error) = writer.flush() {
                eprintln!("[astrcode-log] failed to flush archive log: {error}");
            }
        }
        if let Some(current) = &self.current {
            if let Ok(mut writer) = current.lock() {
                if let Err(error) = writer.flush() {
                    eprintln!("[astrcode-log] failed to flush current log: {error}");
                }
            }
        }
    }
}

/// 写入日志行并立即 flush。写失败时用 eprintln! 明确暴露，不静默吞掉。
/// 不能用 log::error! 因为那会导致递归写入。
fn write_line(writer: &mut BufWriter<File>, channel: &str, line: &str) {
    if let Err(error) = writer.write_all(line.as_bytes()) {
        eprintln!("[astrcode-log] failed to write {channel} log: {error}");
        return;
    }
    if let Err(error) = writer.flush() {
        eprintln!("[astrcode-log] failed to flush {channel} log: {error}");
    }
}

// ── 文件创建 ──────────────────────────────────────────────────────────

/// 解析日志目录路径：`~/.astrcode/logs/`。不存在则递归创建。
fn logs_dir() -> Result<std::path::PathBuf> {
    let dir = astrcode_dir()?.join("logs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 原子创建归档文件。命名：`server-YYYY-MM-DD-HHMMSS-mmm-pid<PID>.log`
/// 若同名已存在（极端碰撞），追加 -1, -2, ... 后缀重试，最多 8 次。
fn create_archive_file(dir: &std::path::Path) -> Result<(File, String)> {
    let now = Local::now();
    let timestamp = now.format("%Y-%m-%d-%H%M%S");
    let millis = now.format("%3f");
    let pid = std::process::id();
    let base = format!("{LOG_FILE_PREFIX}{timestamp}-{millis}-pid{pid}");

    for attempt in 0u32..8 {
        let filename = if attempt == 0 {
            format!("{base}.log")
        } else {
            format!("{base}-{attempt}.log")
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&filename))
        {
            Ok(file) => return Ok((file, filename)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "failed to create archive log '{}': {}",
                    filename,
                    e
                ));
            },
        }
    }
    Err(anyhow::anyhow!(
        "failed to create archive log after 8 retries"
    ))
}

/// 截断重建 server-current.log。每次启动清空旧内容，从头开始写。
fn create_current_file(dir: &std::path::Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dir.join(CURRENT_LOG_NAME))
        .map_err(|e| anyhow::anyhow!("failed to create {}: {}", CURRENT_LOG_NAME, e))
}

/// best-effort 写入 server-latest.txt：一行内容为当前归档文件名。
/// 失败只 eprintln!，不影响初始化。
fn write_latest_link(dir: &std::path::Path, archive_name: &str) {
    if let Err(error) = fs::write(dir.join(LATEST_LINK_NAME), archive_name) {
        eprintln!("[astrcode-log] failed to write {LATEST_LINK_NAME}: {error}");
    }
}

// ── 清理旧归档 ────────────────────────────────────────────────────────

/// 判断文件名是否为归档日志文件（用于清理匹配）
fn is_archive_log_file(name: &str) -> bool {
    name.starts_with(LOG_FILE_PREFIX)
        && name.ends_with(".log")
        && name.contains("-pid")
        && name != CURRENT_LOG_NAME
}

/// 清理旧归档，保留最近 MAX_ARCHIVE_FILES 个。
/// 只删除含 `-pid` 的归档文件，不删 server-current.log 和 server-latest.txt。
/// 调用前提：本次归档文件已在目录中（先创建后清理，计数正确）。
fn cleanup_old_archives(logs_dir: &std::path::Path) {
    let mut entries = match fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(error) => {
            // 非"不存在/权限问题"时报错，方便看清为什么没清理
            if error.kind() != std::io::ErrorKind::NotFound
                && error.kind() != std::io::ErrorKind::PermissionDenied
            {
                eprintln!(
                    "[astrcode-log] failed to read logs directory '{}': {error}",
                    logs_dir.display()
                );
            }
            return;
        },
    };

    let mut archive_files: Vec<_> = (&mut entries)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name_str = name.to_str()?;
            is_archive_log_file(name_str).then(|| entry.path())
        })
        .collect();

    if archive_files.len() <= MAX_ARCHIVE_FILES {
        return;
    }

    // 按文件名排序（含时间戳，字典序 = 时间序）
    archive_files.sort();

    // 删除最老的，仅保留最后 MAX_ARCHIVE_FILES 个
    let to_delete = archive_files.len() - MAX_ARCHIVE_FILES;
    for path in &archive_files[..to_delete] {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::PermissionDenied
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!(
                    "[astrcode-log] failed to remove old archive '{}': {error}",
                    path.display()
                );
            }
        }
    }
}

// ── stderr logger 构建 ────────────────────────────────────────────────

/// 构建 stderr 的 env_logger。
/// Debug：项目 crate info+，依赖 warn+。
/// Release：error+（比完全 Off 更实用）。
fn build_stderr_logger() -> env_logger::Logger {
    let mut builder = env_logger::Builder::new();
    if std::env::var("RUST_LOG").is_ok() {
        builder.parse_default_env();
    } else {
        #[cfg(debug_assertions)]
        {
            for crate_name in PROJECT_CRATE_NAMES {
                builder.filter_module(crate_name, log::LevelFilter::Info);
            }
            builder.filter_level(log::LevelFilter::Warn);
        }
        #[cfg(not(debug_assertions))]
        {
            builder.filter_level(log::LevelFilter::Error);
        }
    }
    builder.format(|buf, record| {
        writeln!(
            buf,
            "[{} {:5}] {}",
            buf.timestamp(),
            record.level(),
            record.args()
        )
    });
    builder.build()
}

#[cfg(debug_assertions)]
const PROJECT_CRATE_NAMES: &[&str] = &[
    "astrcode",
    "astrcode_core",
    "astrcode_runtime",
    "astrcode_runtime_execution",
    "astrcode_runtime_agent_control",
    "astrcode_runtime_agent_loop",
    "astrcode_runtime_agent_tool",
    "astrcode_runtime_config",
    "astrcode_runtime_llm",
    "astrcode_runtime_prompt",
    "astrcode_runtime_session",
    "astrcode_runtime_skill_loader",
    "astrcode_runtime_tool_loader",
    "astrcode_runtime_registry",
    "astrcode_storage",
    "astrcode_protocol",
    "astrcode_server",
    "astrcode_plugin",
    "astrcode_sdk",
];

// ── 入口 ──────────────────────────────────────────────────────────────

/// 退化方案：仅 stderr（文件通道全部失败时使用）。
/// env_logger::Logger 没有 .init()，需手动注册。
fn init_stderr_only_logger() {
    let logger = build_stderr_logger();
    log::set_boxed_logger(Box::new(logger)).expect("failed to set stderr-only logger");
    log::set_max_level(log::LevelFilter::Trace);
}

/// 初始化日志系统。
///
/// 初始化顺序：目录 → 归档(必需) → latest(best-effort) → current(best-effort) → 清理 → 注册
///
/// - Debug 构建：stderr 输出 info+（项目 crate）/ warn+（依赖），与现有行为一致
/// - Release 构建：stderr 输出 error+
/// - 文件通道始终写 warn+，在两种构建中均生效
pub fn init_logger() {
    let logs_dir = match logs_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("[astrcode] failed to create logs directory: {error}");
            init_stderr_only_logger();
            return;
        },
    };

    // 归档文件是持久化底线，必需
    let (archive_file, archive_name) = match create_archive_file(&logs_dir) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[astrcode] failed to create archive log: {error}");
            init_stderr_only_logger();
            return;
        },
    };

    // best-effort：不影响初始化成功
    write_latest_link(&logs_dir, &archive_name);

    let current_file = match create_current_file(&logs_dir) {
        Ok(file) => Some(Mutex::new(BufWriter::new(file))),
        Err(error) => {
            eprintln!("[astrcode] failed to create current log: {error}");
            None
        },
    };

    // 清理旧归档（本次归档已在目录中，计数正确）
    cleanup_old_archives(&logs_dir);

    // 在 move 前保存状态，用于初始化日志消息
    let has_current = current_file.is_some();

    let logger = ServerLogger {
        stderr: build_stderr_logger(),
        archive: Mutex::new(BufWriter::new(archive_file)),
        current: current_file,
    };

    log::set_boxed_logger(Box::new(logger)).expect("failed to set global logger");
    // 让 ServerLogger::enabled() 接收所有消息，各通道自行过滤
    log::set_max_level(log::LevelFilter::Trace);

    // 按 current 实际状态打印初始化结果
    if has_current {
        log::info!(
            "file logging initialized: archive={}, current={}",
            logs_dir.join(&archive_name).display(),
            logs_dir.join(CURRENT_LOG_NAME).display()
        );
    } else {
        log::info!(
            "file logging initialized (archive only, current unavailable): archive={}",
            logs_dir.join(&archive_name).display()
        );
    }
}

// ── 测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_removes_oldest_archives_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        // 创建超过上限的归档文件
        for i in 0..12 {
            let name = format!("server-2026-04-12-{i:02}0000-000-pid12345.log");
            File::create(dir.path().join(&name)).unwrap();
        }
        File::create(dir.path().join("server-current.log")).unwrap();
        File::create(dir.path().join("server-latest.txt")).unwrap();
        File::create(dir.path().join("other.txt")).unwrap();

        cleanup_old_archives(dir.path());

        let archive_count = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.file_name().to_str().map(|n| n.to_string()))
                    .map(|n| is_archive_log_file(&n))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(archive_count, MAX_ARCHIVE_FILES);
    }

    #[test]
    fn cleanup_ignores_current_latest_and_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..12 {
            let name = format!("server-2026-04-12-{i:02}0000-000-pid12345.log");
            File::create(dir.path().join(&name)).unwrap();
        }
        File::create(dir.path().join("server-current.log")).unwrap();
        File::create(dir.path().join("server-latest.txt")).unwrap();
        File::create(dir.path().join("other.txt")).unwrap();

        cleanup_old_archives(dir.path());

        assert!(dir.path().join("server-current.log").exists());
        assert!(dir.path().join("server-latest.txt").exists());
        assert!(dir.path().join("other.txt").exists());
    }

    #[test]
    fn cleanup_does_nothing_when_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            let name = format!("server-2026-04-12-{i:02}0000-000-pid12345.log");
            File::create(dir.path().join(&name)).unwrap();
        }
        cleanup_old_archives(dir.path());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 5);
    }

    #[test]
    fn cleanup_handles_missing_dir_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        // 不应 panic
        cleanup_old_archives(&missing);
    }

    #[test]
    fn archive_file_retries_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let pid = std::process::id();
        let now = Local::now();
        // 覆盖当前秒和接下来 3 秒的所有毫秒候选文件名，
        // 确保 create_archive_file 无论在哪一秒执行都会碰到已占文件名
        for sec_offset in 0..=3i64 {
            let t = now + chrono::Duration::seconds(sec_offset);
            let timestamp = t.format("%Y-%m-%d-%H%M%S");
            for ms in 0..1000u32 {
                let name = format!("server-{timestamp}-{ms:03}-pid{pid}.log");
                File::create(dir.path().join(&name)).unwrap();
            }
        }
        // 所有首选候选都被占，create_archive_file 必须走 -N 后缀重试
        let (_file, retried_name) = create_archive_file(dir.path()).unwrap();
        assert!(
            retried_name.contains("-1.log")
                || retried_name.contains("-2.log")
                || retried_name.contains("-3.log"),
            "expected retry suffix after blocking all primary candidates, got: {retried_name}"
        );
    }

    #[test]
    fn current_file_is_truncated_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        // 先写入旧内容
        fs::write(dir.path().join(CURRENT_LOG_NAME), "old content").unwrap();
        // 重新创建应截断
        let file = create_current_file(dir.path()).unwrap();
        drop(file);
        let content = fs::read_to_string(dir.path().join(CURRENT_LOG_NAME)).unwrap();
        assert!(content.is_empty());
    }
}
