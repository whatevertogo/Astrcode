//! 流式执行与事件发射。
//!
//! 本模块提供流式能力调用的基础设施：
//!
//! - `EventEmitter`: 异步事件发射器，用于在能力执行过程中发送增量事件
//! - `StreamExecution`: 流式执行的接收端，封装了 `mpsc::UnboundedReceiver<EventMessage>`
//!
//! ## 使用场景
//!
//! 当插件能力需要逐步输出结果（如代码生成的增量 patch、搜索工具的逐步结果）时，
//! 通过 `EventEmitter::delta()` 发送事件，调用方通过 `StreamExecution::recv()` 接收。

use std::{future::Future, pin::Pin, sync::Arc};

use astrcode_core::Result;
use astrcode_protocol::plugin::EventMessage;
use serde_json::Value;
use tokio::sync::mpsc;

type EmitFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type EmitFn = dyn Fn(String, Value) -> EmitFuture + Send + Sync;

/// 异步事件发射器。
///
/// 用于在能力执行过程中发送增量事件（delta events）。
/// 内部使用类型擦除（type erasure）存储异步闭包，避免泛型污染 API。
///
/// # 设计选择
///
/// - 使用 `Option<Arc<EmitFn>>` 而非直接存储闭包，使得 `Default` 实现为 no-op
/// - `Clone` 实现共享同一个 emit 函数，适合在多个地方传递
/// - `noop()` 构造函数创建一个不执行任何操作的发射器，用于一元调用场景
#[derive(Clone, Default)]
pub struct EventEmitter {
    emit: Option<Arc<EmitFn>>,
}

impl EventEmitter {
    /// 创建新的事件发射器。
    ///
    /// 接受一个异步闭包 `(event_name, payload) -> Future<Output = Result<()>>`，
    /// 每次调用 `delta()` 时执行该闭包。
    pub fn new<F, Fut>(emit: F) -> Self
    where
        F: Fn(String, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            emit: Some(Arc::new(move |event, payload| {
                Box::pin(emit(event, payload))
            })),
        }
    }

    /// 创建不执行任何操作的事件发射器。
    ///
    /// 用于一元调用场景，能力处理器不需要发送增量事件。
    pub fn noop() -> Self {
        Self { emit: None }
    }

    /// 发送一个增量事件。
    ///
    /// 如果发射器是 no-op 模式（通过 `Default` 或 `noop()` 创建），
    /// 则直接返回 `Ok(())`，不执行任何操作。
    pub async fn delta(&self, event: impl Into<String>, payload: Value) -> Result<()> {
        match &self.emit {
            Some(emit) => emit(event.into(), payload).await,
            None => Ok(()),
        }
    }
}

/// 流式执行的接收端。
///
/// 封装了 `mpsc::UnboundedReceiver<EventMessage>`，提供类型安全的
/// 流式事件接收接口。由 `Peer::invoke_stream()` 创建并返回。
///
/// # 事件序列
///
/// 典型的流式执行会产生以下事件序列：
/// 1. `EventPhase::Started` — 调用开始
/// 2. `EventPhase::Delta` × N — 增量输出
/// 3. `EventPhase::Completed` 或 `EventPhase::Failed` — 终端事件
///
/// 收到终端事件后，channel 可能还会关闭（返回 `None`），
/// 调用方应同时处理终端事件和 channel 关闭两种情况。
pub struct StreamExecution {
    request_id: String,
    receiver: mpsc::UnboundedReceiver<EventMessage>,
}

impl StreamExecution {
    /// 创建新的流式执行实例。
    pub fn new(
        request_id: impl Into<String>,
        receiver: mpsc::UnboundedReceiver<EventMessage>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            receiver,
        }
    }

    /// 获取关联的请求 ID。
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// 接收下一个事件。
    ///
    /// 异步阻塞直到有新事件到达或 channel 关闭。
    /// 返回 `None` 表示 channel 已关闭（发送端已丢弃）。
    pub async fn recv(&mut self) -> Option<EventMessage> {
        self.receiver.recv().await
    }
}
