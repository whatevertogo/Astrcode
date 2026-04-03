//! 标准输入输出传输实现。
//!
//! 通过 stdio 管道在宿主进程和插件进程之间传输 JSON-RPC 消息。
//!
//! ## 协议
//!
//! 每条消息是一个 JSON 字符串，以换行符（`\n`）结尾。
//! 接收端按行读取，自动去除行尾的 `\r` 和 `\n`。
//!
//! ## 线程安全
//!
//! `writer` 和 `reader` 分别使用独立的 `Mutex` 保护，
//! 允许并发发送和接收（但不能同时有多个发送或接收）。

use std::pin::Pin;

use async_trait::async_trait;
use tokio::{
    io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::Mutex,
};

use super::Transport;

/// 基于标准输入输出的传输实现。
///
/// 支持两种模式：
/// - **子进程模式** (`from_child`): 宿主进程管理子进程的 stdin/stdout
/// - **进程内模式** (`from_process_stdio`): 插件进程使用自己的 stdin/stdout 与宿主通信
///
/// # 注意
///
/// `writer` 和 `reader` 使用 `Pin<Box<dyn ...>>` 而非具体类型，
/// 因为两种模式使用的底层类型不同（`ChildStdin` vs `io::stdout`）。
pub struct StdioTransport {
    writer: Mutex<Pin<Box<dyn AsyncWrite + Send>>>,
    reader: Mutex<Pin<Box<dyn AsyncBufRead + Send>>>,
}

impl StdioTransport {
    /// 从子进程的 stdin/stdout 创建传输。
    ///
    /// 用于宿主进程模式：宿主拥有子进程的 stdin/stdout 句柄，
    /// 通过写入 stdin 发送消息，从 stdout 读取消息。
    pub fn from_child(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            writer: Mutex::new(Box::pin(stdin)),
            reader: Mutex::new(Box::pin(BufReader::new(stdout))),
        }
    }

    /// 从当前进程的标准输入输出创建传输。
    ///
    /// 用于插件进程模式：插件作为子进程运行，使用自己的 stdin/stdout
    /// 与宿主通信。写入 stdout 发送消息给宿主，从 stdin 读取宿主消息。
    ///
    /// # 注意
    ///
    /// 方向是反直觉的：插件写入 stdout → 宿主从 stdout 读取，
    /// 这是因为 stdout 是管道的一端，另一端由宿主持有。
    pub fn from_process_stdio() -> Self {
        Self {
            writer: Mutex::new(Box::pin(io::stdout())),
            reader: Mutex::new(Box::pin(BufReader::new(io::stdin()))),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    /// 发送一条消息。
    ///
    /// 将 payload 写入底层输出，追加换行符并 flush。
    /// flush 确保消息立即通过管道传递到对端。
    async fn send(&self, payload: &str) -> Result<(), String> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|error| format!("failed to write plugin payload: {error}"))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|error| format!("failed to terminate plugin payload: {error}"))?;
        writer
            .flush()
            .await
            .map_err(|error| format!("failed to flush plugin payload: {error}"))
    }

    /// 接收一条消息。
    ///
    /// 按行读取，自动去除行尾的 `\r` 和 `\n`。
    /// 返回 `None` 表示输入流已关闭（对端退出或管道断裂）。
    async fn recv(&self) -> Result<Option<String>, String> {
        let mut reader = self.reader.lock().await;
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("failed to read plugin payload: {error}"))?;
        if bytes == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
    }
}
