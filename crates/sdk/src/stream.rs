//! 流式响应写入器。
//!
//! 本模块提供 `StreamWriter`，用于工具在执行过程中向客户端发送增量输出。
//!
//! ## 使用场景
//!
//! - **长耗时工具**: shell 命令的 stdout/stderr 逐行输出
//! - **LLM 流式响应**: token 级别的文本增量
//! - **文件编辑**: diff 补丁的逐步应用
//!
//! ## 设计意图
//!
//! 工具不应等到全部完成才返回结果，而是通过流式输出让用户
//! 实时看到进度。`StreamWriter` 将每个增量事件持久化并广播，
//! 确保断线重连后可通过 replay 恢复。
//!
//! ## 线程安全
//!
//! `StreamWriter` 是 `Clone` 且线程安全的，可在 async 任务中
//! 跨多个 future 共享，适合并发发送多个流式事件。

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::SdkError;

type StreamResult<T> = Result<T, SdkError>;
type StreamCallback = dyn Fn(StreamChunk) -> StreamResult<()> + Send + Sync;

/// 单个流式事件块。
///
/// 表示工具执行过程中产生的一个增量输出事件，
/// 包含事件类型（如 "message.delta"）和负载数据。
///
/// ## 事件命名约定
///
/// 事件名使用点分格式，如 `message.delta`、`artifact.patch`，
/// 前端据此选择对应的渲染组件。
#[derive(Debug, Clone, PartialEq)]
pub struct StreamChunk {
    /// 事件类型名称。
    pub event: String,
    /// 事件负载，结构由事件类型决定。
    pub payload: Value,
}

/// 流式响应写入器。
///
/// 工具通过此对象在执行过程中发送增量输出到客户端。
///
/// ## 线程安全与克隆
///
/// `StreamWriter` 内部使用 `Arc<Mutex<>>` 共享状态，
/// 因此 `Clone` 是廉价的引用计数增加，可在 async 任务间安全共享。
///
/// ## 回调机制
///
/// 可通过 `with_callback` 注册回调函数，每次 `emit` 时触发。
/// 回调用于将事件推送到运行时广播通道或持久化层。
/// 如果不设置回调，`emit` 仅记录到内部缓冲区（可通过 `records()` 读取）。
///
/// ## 为什么同时有 records 和 callback
///
/// - `records`: 用于测试断言和断线重连时的历史回放
/// - `callback`: 用于实时推送到运行时广播通道
///
/// 两者互补，不是冗余设计。
#[derive(Clone, Default)]
pub struct StreamWriter {
    records: Arc<Mutex<Vec<StreamChunk>>>,
    callback: Option<Arc<StreamCallback>>,
}

impl StreamWriter {
    /// 创建带回调的流式写入器。
    ///
    /// 回调函数会在每次 `emit` 时被调用，
    /// 通常用于将事件推送到运行时的广播通道。
    pub fn with_callback<F>(callback: F) -> Self
    where
        F: Fn(StreamChunk) -> StreamResult<()> + Send + Sync + 'static,
    {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            callback: Some(Arc::new(callback)),
        }
    }

    /// 发送一个流式事件。
    ///
    /// 事件会被记录到内部缓冲区（可通过 `records()` 读取），
    /// 如果设置了回调，还会调用回调函数进行实时推送。
    ///
    /// ## 错误处理
    ///
    /// 如果回调返回错误，`emit` 会将其包装为 `SdkError::StreamEmit` 返回，
    /// 工具应据此决定是否中止执行。
    pub fn emit(&self, event: impl Into<String>, payload: Value) -> StreamResult<()> {
        let event = event.into();
        let chunk = StreamChunk {
            event: event.clone(),
            payload,
        };
        self.records
            .lock()
            .map_err(|_| SdkError::internal("stream records lock poisoned"))?
            .push(chunk.clone());
        if let Some(callback) = &self.callback {
            callback(chunk).map_err(|error| SdkError::StreamEmit {
                event,
                message: error.to_string(),
                details: error.details(),
            })?;
        }
        Ok(())
    }

    /// 发送文本增量事件。
    ///
    /// 便捷方法，等价于 `emit("message.delta", { "text": ... })`，
    /// 用于 LLM 流式响应等场景的逐段文本输出。
    pub fn message_delta(&self, text: impl Into<String>) -> StreamResult<()> {
        self.emit("message.delta", json!({ "text": text.into() }))
    }

    /// 发送文件补丁事件。
    ///
    /// 便捷方法，等价于 `emit("artifact.patch", { "path": ..., "patch": ... })`，
    /// 用于工具逐步应用文件修改时向前端发送 diff。
    pub fn artifact_patch(
        &self,
        path: impl Into<String>,
        patch: impl Into<String>,
    ) -> StreamResult<()> {
        self.emit(
            "artifact.patch",
            json!({
                "path": path.into(),
                "patch": patch.into(),
            }),
        )
    }

    /// 发送诊断信息事件。
    ///
    /// 便捷方法，等价于 `emit("diagnostic", { "severity": ..., "message": ... })`，
    /// 用于工具在执行过程中向前端报告警告、错误等诊断信息。
    pub fn diagnostic(
        &self,
        severity: impl Into<String>,
        message: impl Into<String>,
    ) -> StreamResult<()> {
        self.emit(
            "diagnostic",
            json!({
                "severity": severity.into(),
                "message": message.into(),
            }),
        )
    }

    /// 返回所有已记录的流式事件。
    ///
    /// 用于测试断言、断线重连时的历史回放，
    /// 或工具执行完成后审计发送了哪些事件。
    ///
    /// ## 线程安全
    ///
    /// 获取锁时会短暂阻塞，如果锁被毒化（panic）则返回错误。
    pub fn records(&self) -> StreamResult<Vec<StreamChunk>> {
        self.records
            .lock()
            .map_err(|_| SdkError::internal("stream records lock poisoned"))
            .map(|guard| guard.clone())
    }
}
