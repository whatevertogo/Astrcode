//! # Stdio 传输实现
//!
//! 通过 `tokio::process::Command` 启动子进程，使用 stdin/stdout 进行 JSON-RPC 通信。
//! 按 SIGINT → SIGTERM → SIGKILL 顺序优雅关闭子进程。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use astrcode_core::{AstrError, Result};
use async_trait::async_trait;
use log::info;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

use super::McpTransport;
use crate::protocol::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// stdio 传输：通过子进程的 stdin/stdout 进行 JSON-RPC 通信。
pub struct StdioTransport {
    /// 子进程句柄。
    child: Option<Child>,
    /// 子进程 stdin（Mutex 包装以支持 &self 下的异步写入）。
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    /// 子进程 stdout 行读取器。
    stdout: Option<Arc<Mutex<Lines<BufReader<ChildStdout>>>>>,
    /// 传输是否活跃。
    active: Arc<AtomicBool>,
    /// 启动命令。
    command: String,
    /// 启动参数。
    args: Vec<String>,
    /// 子进程环境变量。
    env: Vec<(String, String)>,
}

impl StdioTransport {
    /// 创建 stdio 传输。
    pub fn new(command: impl Into<String>, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            active: Arc::new(AtomicBool::new(false)),
            command: command.into(),
            args,
            env,
        }
    }

    #[cfg(unix)]
    fn send_unix_signal(child: &Child, signal: i32, signal_name: &str) {
        let Some(pid) = child.id() else {
            info!(
                "skip sending {} to MCP server because the process id is unavailable",
                signal_name
            );
            return;
        };

        // libc::kill 是 Unix 发送进程信号的底层系统调用，这里只在 unix 平台使用。
        let result = unsafe { libc::kill(pid as i32, signal) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            info!(
                "failed to send {} to MCP server pid {}: {}",
                signal_name, pid, error
            );
        }
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn start(&mut self) -> Result<()> {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // 注入环境变量
        for (key, value) in &self.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            AstrError::io(format!("failed to spawn MCP server: {}", self.command), e)
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AstrError::Internal("failed to open stdin for MCP server".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AstrError::Internal("failed to open stdout for MCP server".into()))?;

        self.stdin = Some(Arc::new(Mutex::new(stdin)));
        self.stdout = Some(Arc::new(Mutex::new(BufReader::new(stdout).lines())));
        self.child = Some(child);
        self.active.store(true, Ordering::SeqCst);

        info!(
            "MCP stdio transport started: {} {}",
            self.command,
            self.args.join(" ")
        );

        Ok(())
    }

    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let stdin_arc = self
            .stdin
            .as_ref()
            .ok_or_else(|| AstrError::Internal("stdio transport not started".into()))?;

        // 序列化请求并写入 stdin，每条消息一行
        let mut json = serde_json::to_string(&request)
            .map_err(|e| AstrError::parse("serialize JSON-RPC request", e))?;
        json.push('\n');

        {
            let mut stdin = stdin_arc.lock().await;
            stdin
                .write_all(json.as_bytes())
                .await
                .map_err(|e| AstrError::io("write to MCP stdin", e))?;
            stdin
                .flush()
                .await
                .map_err(|e| AstrError::io("flush MCP stdin", e))?;
        }

        // 从 stdout 读取响应行
        let stdout_arc = self
            .stdout
            .as_ref()
            .ok_or_else(|| AstrError::Internal("stdio transport not started".into()))?;
        let mut stdout = stdout_arc.lock().await;

        loop {
            let line = stdout
                .next_line()
                .await
                .map_err(|e| AstrError::io("read from MCP stdout", e))?
                .ok_or_else(|| {
                    self.active.store(false, Ordering::SeqCst);
                    AstrError::Network("MCP server closed stdout".into())
                })?;

            // 尝试解析为 JSON-RPC 响应，跳过非 JSON 行
            if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&line) {
                return Ok(response);
            }
            // 非 JSON-RPC 行被跳过（服务器可能输出日志到 stdout）
        }
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()> {
        let stdin_arc = self
            .stdin
            .as_ref()
            .ok_or_else(|| AstrError::Internal("stdio transport not started".into()))?;

        let mut json = serde_json::to_string(&notification)
            .map_err(|e| AstrError::parse("serialize JSON-RPC notification", e))?;
        json.push('\n');

        let mut stdin = stdin_arc.lock().await;
        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(|e| AstrError::io("write notification to MCP stdin", e))?;
        stdin
            .flush()
            .await
            .map_err(|e| AstrError::io("flush MCP stdin", e))?;

        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.active.store(false, Ordering::SeqCst);

        // 先关闭 stdin，让子进程感知 EOF
        self.stdin.take();

        if let Some(mut child) = self.child.take() {
            // Windows 直接 kill
            #[cfg(windows)]
            {
                let _ = child.kill().await;
            }

            #[cfg(not(windows))]
            {
                // 优雅关闭：SIGINT → 等 5s → SIGTERM → 等 5s → SIGKILL
                use tokio::time::{Duration, timeout};

                // SIGINT
                Self::send_unix_signal(&child, libc::SIGINT, "SIGINT");

                match timeout(Duration::from_secs(5), child.wait()).await {
                    Ok(Ok(_)) => {
                        info!("MCP server exited gracefully after SIGINT");
                        return Ok(());
                    },
                    _ => {},
                }

                // SIGTERM
                Self::send_unix_signal(&child, libc::SIGTERM, "SIGTERM");

                match timeout(Duration::from_secs(5), child.wait()).await {
                    Ok(Ok(_)) => {
                        info!("MCP server exited after SIGTERM");
                        return Ok(());
                    },
                    _ => {},
                }

                // SIGKILL
                let _ = child.kill().await;
                info!("MCP server killed after SIGKILL");
            }

            let _ = child.wait().await;
        }

        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn transport_type(&self) -> &'static str {
        "stdio"
    }
}
