//! 插件进程管理。
//!
//! 本模块负责启动、监控和关闭插件子进程。
//!
//! ## 进程生命周期
//!
//! 1. `PluginProcess::start()` — 根据 manifest 启动子进程，创建 stdio 传输
//! 2. `status()` — 非阻塞检查进程状态
//! 3. `shutdown()` — 终止子进程
//!
//! ## 传输层
//!
//! 进程启动后立即创建 `StdioTransport`，将子进程的 stdin/stdout
//! 包装为异步传输层，供 `Peer` 使用。

use std::{process::Stdio, sync::Arc};

use astrcode_core::{AstrError, PluginManifest, Result};
use tokio::process::{Child, Command};

use crate::transport::{StdioTransport, Transport};

/// 插件子进程。
///
/// 封装了 tokio 子进程和对应的 stdio 传输层。
/// 由 `PluginProcess::start()` 创建，由 `Supervisor` 管理生命周期。
pub struct PluginProcess {
    pub manifest: PluginManifest,
    pub child: Child,
    transport: Arc<dyn Transport>,
}

/// 插件进程的运行状态。
///
/// 由 `PluginProcess::status()` 返回，通过 `try_wait()` 非阻塞获取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProcessStatus {
    pub running: bool,
    pub exit_code: Option<i32>,
}

impl PluginProcess {
    /// 根据清单启动插件子进程。
    ///
    /// # 流程
    ///
    /// 1. 从 manifest 获取可执行文件路径（必须存在）
    /// 2. 配置命令参数和工作目录
    /// 3. 设置 stdin/stdout 为管道模式（用于 JSON-RPC 通信）
    /// 4. 生成子进程
    /// 5. 取出 stdin/stdout 句柄创建 `StdioTransport`
    ///
    /// # 错误
    ///
    /// - manifest 没有 `executable` → `Validation` 错误
    /// - 子进程生成失败 → `Io` 错误
    /// - stdin/stdout 不可用 → `Internal` 错误（理论上不应发生）
    pub async fn start(manifest: &PluginManifest) -> Result<Self> {
        let executable = manifest.executable.as_ref().ok_or_else(|| {
            AstrError::Validation(format!("plugin '{}' has no executable", manifest.name))
        })?;
        let mut command = Command::new(executable);
        command
            .args(&manifest.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if let Some(working_dir) = &manifest.working_dir {
            command.current_dir(working_dir);
        }
        let mut child = command.spawn().map_err(|error| {
            AstrError::io(format!("failed to spawn plugin '{executable}'"), error)
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AstrError::Internal(format!("plugin '{}' did not expose stdin", manifest.name))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AstrError::Internal(format!("plugin '{}' did not expose stdout", manifest.name))
        })?;
        let transport: Arc<dyn Transport> = Arc::new(StdioTransport::from_child(stdin, stdout));

        Ok(Self {
            manifest: manifest.clone(),
            child,
            transport,
        })
    }

    /// 获取传输层的引用。
    ///
    /// 返回 `Arc` 克隆，可与 `Peer` 共享。
    pub fn transport(&self) -> Arc<dyn Transport> {
        Arc::clone(&self.transport)
    }

    /// 非阻塞检查进程状态。
    ///
    /// 使用 `try_wait()` 检查进程是否已退出，不会阻塞等待。
    pub fn status(&mut self) -> Result<PluginProcessStatus> {
        let exit_status = self
            .child
            .try_wait()
            .map_err(|error| AstrError::io("failed to poll plugin process", error))?;
        Ok(match exit_status {
            Some(status) => PluginProcessStatus {
                running: false,
                exit_code: status.code(),
            },
            None => PluginProcessStatus {
                running: true,
                exit_code: None,
            },
        })
    }

    /// 终止插件子进程。
    ///
    /// 调用 `kill()` 强制终止进程。
    ///
    /// # 容错
    ///
    /// 如果进程已经退出（`InvalidInput` 错误），视为成功。
    /// 这是为了避免在进程已退出的情况下重复关闭导致错误。
    pub async fn shutdown(&mut self) -> Result<()> {
        match self.child.kill().await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(AstrError::io("failed to terminate plugin process", error)),
        }
    }
}
