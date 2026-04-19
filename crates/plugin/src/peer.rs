//! 插件对等体（Peer）—— 管理与插件进程的双向 JSON-RPC 通信。
//!
//! ## 核心职责
//!
//! `Peer` 是宿主进程与插件进程之间的通信桥梁，负责：
//! - 发送 `InitializeMessage` 完成握手协商
//! - 发送 `InvokeMessage` 调用插件能力并等待 `ResultMessage`
//! - 发送 `InvokeMessage(stream=true)` 获取流式 `StreamExecution`
//! - 接收插件主动发起的能力调用（host-to-plugin 反向调用）
//! - 处理 `CancelMessage` 取消请求
//! - 管理后台读循环和所有活跃请求的生命周期
//!
//! ## 消息流
//!
//! ```text
//! 宿主 (Peer)                              插件进程
//! ──────────────                          ──────────────
//! InitializeMessage ──────────────────►
//!            ◄────────────────────── ResultMessage (initialize)
//!
//! InvokeMessage ─────────────────────►
//!            ◄────────────────────── ResultMessage (unary)
//!
//! InvokeMessage(stream=true) ────────►
//!            ◄────────────────────── EventMessage (started)
//!            ◄────────────────────── EventMessage (delta) × N
//!            ◄────────────────────── EventMessage (completed/failed)
//!
//!            ◄────────────────────── InvokeMessage (插件→宿主)
//! InvokeMessage ─────────────────────► (宿主处理)
//!            ◄────────────────────── ResultMessage
//!
//! CancelMessage ─────────────────────►
//! ```
//!
//! ## 同步原语选择
//!
//! `read_loop_handle` 和 `invoke_handles` 使用 `std::sync::Mutex`（非 tokio Mutex），
//! 因为这些字段只在短时间内持有锁（插入/取出 HashMap 条目），不需要跨 await 点。
//! 使用 `std::sync::Mutex` 避免了 tokio Mutex 的额外开销和潜在的 "mutex held across await" 警告。

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{AstrError, CancelToken, Result};
use astrcode_protocol::plugin::{
    CancelMessage, ErrorPayload, EventMessage, EventPhase, InitializeMessage, InitializeResultData,
    InvokeMessage, PROTOCOL_VERSION, PluginMessage, ResultMessage,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use crate::{CapabilityRouter, EventEmitter, StreamExecution, transport::Transport};

/// 与插件进程的双向通信端。
///
/// `Peer` 封装了与单个插件进程的完整 JSON-RPC 生命周期，包括握手、
/// 请求-响应、流式事件、取消和优雅关闭。
///
/// ## 架构概览
///
/// ```text
/// Host (本进程)                          Plugin (子进程)
/// ──────────────                        ──────────────
/// invoke() ─── InvokeMessage ──────►  处理请求
///            ◄──── ResultMessage ───────  返回结果
///            ◄──── EventMessage ────────  流式增量
///
/// read_loop ◄──── PluginMessage ───────  所有入站消息
///            ─── CancelMessage ──────►  取消请求
/// ```
///
/// ## 关键状态
///
/// - `pending_results`: 等待结果的一次性 channel，invoke 时插入，收到 ResultMessage 时取出
/// - `pending_streams`: 流式调用的增量 channel，invoke(stream=true) 时插入
/// - `inbound_cancellations`: 插件调用 host 能力时的取消令牌，host 可以取消
/// - `read_loop_handle`: 后台读循环的 JoinHandle，abort() 时用于取消
/// - `invoke_handles`: 每个入站 invoke 对应的处理任务，abort() 时批量取消
///
/// ## 同步原语选择
///
/// `read_loop_handle` 和 `invoke_handles` 使用 `std::sync::Mutex`（非 tokio Mutex），
/// 因为这些字段只在短时间内持有锁（插入/取出 HashMap 条目），不需要跨 await 点。
#[derive(Clone)]
pub struct Peer {
    inner: Arc<PeerInner>,
}

struct PeerInner {
    transport: Arc<dyn Transport>,
    local_initialize: InitializeMessage,
    router: Arc<CapabilityRouter>,
    pending_results: Mutex<HashMap<String, oneshot::Sender<ResultMessage>>>,
    pending_streams: Mutex<HashMap<String, mpsc::UnboundedSender<EventMessage>>>,
    inbound_cancellations: Mutex<HashMap<String, CancelToken>>,
    remote_initialize: Mutex<Option<InitializeResultData>>,
    closed_reason: Mutex<Option<String>>,
    closed_notify: Notify,
    read_loop_handle: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    invoke_handles: std::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl Peer {
    /// 创建新的 Peer 并启动后台读循环。
    ///
    /// # 参数
    ///
    /// * `transport` - 底层传输层（通常是 stdio），负责序列化和发送/接收 JSON-RPC 消息
    /// * `local_initialize` - 本地初始化信息，包含本端支持的能力和配置
    /// * `router` - 能力路由器，用于处理插件反向调用宿主能力的请求
    ///
    /// # 注意
    ///
    /// 构造完成后立即启动后台读循环（`spawn_read_loop`），
    /// 该循环会持续监听来自插件的入站消息。
    pub fn new(
        transport: Arc<dyn Transport>,
        local_initialize: InitializeMessage,
        router: Arc<CapabilityRouter>,
    ) -> Self {
        let inner = Arc::new(PeerInner {
            transport,
            local_initialize,
            router,
            pending_results: Mutex::new(HashMap::new()),
            pending_streams: Mutex::new(HashMap::new()),
            inbound_cancellations: Mutex::new(HashMap::new()),
            remote_initialize: Mutex::new(None),
            closed_reason: Mutex::new(None),
            closed_notify: Notify::new(),
            read_loop_handle: std::sync::Mutex::new(None),
            invoke_handles: std::sync::Mutex::new(HashMap::new()),
        });

        let peer = Self { inner };
        peer.spawn_read_loop();
        peer
    }

    /// 发送初始化请求并等待插件响应，完成协议版本协商。
    ///
    /// 这是与插件通信的第一步。发送 `InitializeMessage` 后等待 `ResultMessage`，
    /// 解析返回的 `InitializeResultData` 并验证协议版本兼容性。
    ///
    /// # 返回
    ///
    /// 返回协商后的 `InitializeResultData`，包含插件声明的能力列表、
    /// 支持的 profiles 和元数据。
    ///
    /// # 错误
    ///
    /// - 返回的消息类型不是 `initialize` 时返回内部错误
    /// - 插件返回 `success: false` 时返回对应的错误载荷
    /// - 解析 JSON 失败时返回验证错误
    pub async fn initialize(&self) -> Result<InitializeResultData> {
        let request = self.inner.local_initialize.clone();
        let response = self.await_result(request).await?;
        if response.kind.as_deref() != Some("initialize") {
            return Err(AstrError::Internal(format!(
                "expected initialize result for '{}', received kind {:?}",
                response.id, response.kind
            )));
        }
        if !response.success {
            return Err(result_error_to_astr(response));
        }
        let negotiated: InitializeResultData = response.parse_output().map_err(|error| {
            AstrError::Validation(format!("failed to parse initialize result: {error}"))
        })?;
        *self.inner.remote_initialize.lock().await = Some(negotiated.clone());
        Ok(negotiated)
    }

    /// 调用插件的某个能力并等待完整结果。
    ///
    /// 发送 `InvokeMessage`（`stream=false`），注册一个 oneshot channel 到
    /// `pending_results`，然后等待对应的 `ResultMessage` 返回。
    ///
    /// # 参数
    ///
    /// * `request` - 调用请求，包含能力名称、输入参数和上下文
    ///
    /// # 返回
    ///
    /// 返回 `ResultMessage`，调用者需自行检查 `success` 字段判断成功与否。
    pub async fn invoke(&self, request: InvokeMessage) -> Result<ResultMessage> {
        self.await_result(request).await
    }

    /// 以流式模式调用插件的某个能力。
    ///
    /// 与 `invoke()` 不同，此方法不会等待完整结果，而是返回一个 `StreamExecution`，
    /// 调用者可以通过 `recv()` 逐步接收 `EventMessage` 增量事件。
    ///
    /// # 流程
    ///
    /// 1. 创建无界 channel，将 sender 注册到 `pending_streams`
    /// 2. 发送 `InvokeMessage(stream=true)`
    /// 3. 如果发送失败，清理已注册的 channel 并返回错误
    /// 4. 发送成功则返回 `StreamExecution`，包含 receiver 和 request_id
    ///
    /// # 注意
    ///
    /// 流式事件（started → delta × N → completed/failed）会通过
    /// `handle_event` 路由到对应的 channel。终端事件（completed/failed）
    /// 到达后会自动从 `pending_streams` 中移除。
    pub async fn invoke_stream(&self, request: InvokeMessage) -> Result<StreamExecution> {
        let request_id = request.id.clone();
        let (sender, receiver) = mpsc::unbounded_channel();
        self.inner
            .pending_streams
            .lock()
            .await
            .insert(request_id.clone(), sender);
        let send_result = self.send_message(&PluginMessage::Invoke(request)).await;
        if let Err(error) = send_result {
            self.inner.pending_streams.lock().await.remove(&request_id);
            return Err(error);
        }
        Ok(StreamExecution::new(request_id, receiver))
    }

    /// 取消一个正在进行的请求。
    ///
    /// 发送 `CancelMessage` 到插件进程。插件收到后应停止当前操作
    /// 并返回一个 failed 的终端事件。
    ///
    /// # 参数
    ///
    /// * `request_id` - 要取消的请求 ID
    /// * `reason` - 可选的取消原因，用于日志和调试
    pub async fn cancel(
        &self,
        request_id: impl Into<String>,
        reason: Option<String>,
    ) -> Result<()> {
        self.send_message(&PluginMessage::Cancel(CancelMessage {
            id: request_id.into(),
            reason,
        }))
        .await
    }

    /// 获取插件在握手时返回的初始化结果。
    ///
    /// 返回 `None` 表示尚未完成 `initialize()` 调用。
    pub async fn remote_initialize(&self) -> Option<InitializeResultData> {
        self.inner.remote_initialize.lock().await.clone()
    }

    /// 获取 Peer 关闭的原因。
    ///
    /// 返回 `None` 表示 Peer 仍在正常运行。
    /// 一旦关闭原因被设置，Peer 将不再处理任何新消息。
    pub async fn closed_reason(&self) -> Option<String> {
        self.inner.closed_reason.lock().await.clone()
    }

    /// 等待 Peer 关闭。
    ///
    /// 异步阻塞直到 `closed_reason` 被设置。使用 `Notify` 避免忙等待，
    /// 每次被唤醒后重新检查条件以处理虚假唤醒。
    pub async fn wait_closed(&self) {
        loop {
            let notified = self.inner.closed_notify.notified();
            if self.inner.closed_reason.lock().await.is_some() {
                return;
            }
            notified.await;
        }
    }

    /// 启动后台读循环，持续监听来自插件的入站消息。
    ///
    /// 读循环在独立的 tokio task 中运行，负责：
    /// - 从传输层读取原始 JSON 字符串
    /// - 反序列化为 `PluginMessage`
    /// - 分发到对应的处理器（handle_*）
    ///
    /// 如果读循环因任何原因退出（传输关闭、解析错误等），
    /// 会触发 `close()` 并通知所有等待方。
    fn spawn_read_loop(&self) {
        let inner = Arc::clone(&self.inner);
        let handle = tokio::spawn(async move {
            inner.read_loop().await;
        });
        // Store the handle so we can abort the read loop during shutdown.
        // Using std::sync::Mutex is safe here: the write happens synchronously
        // during Peer::new, and abort() reads it from an async context with
        // negligible contention.
        astrcode_core::support::with_lock_recovery(
            &self.inner.read_loop_handle,
            "peer.read_loop_handle",
            |guard| *guard = Some(handle),
        );
    }

    /// 中止读循环和所有活跃的入站 invoke 处理器。
    ///
    /// 由 `Supervisor` 在关闭时调用，确保 peer 的后台任务不会在进程终止后继续运行。
    ///
    /// # 清理顺序
    ///
    /// 1. 中止读循环（停止接收新消息）
    /// 2. 中止所有入站 invoke 处理器（插件→宿主的调用）
    /// 3. 取消所有入站请求的取消令牌
    /// 4. 设置关闭原因，通知所有等待方
    pub async fn abort(&self) {
        // 使用 into_inner() 处理可能的 poison，避免在清理路径中 panic
        if let Some(handle) = self
            .inner
            .read_loop_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
        let handles = std::mem::take(
            &mut *self
                .inner
                .invoke_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for (_, handle) in handles {
            handle.abort();
        }
        let cancellations = std::mem::take(&mut *self.inner.inbound_cancellations.lock().await);
        for (_, cancel) in cancellations {
            cancel.cancel();
        }
        self.inner
            .close("peer aborted during shutdown".to_string())
            .await;
    }

    /// 发送请求并等待对应的结果。
    ///
    /// 这是 `invoke()` 和 `initialize()` 的底层实现。
    ///
    /// # 流程
    ///
    /// 1. 将请求转换为 `PluginMessage`
    /// 2. 创建 oneshot channel，将 sender 注册到 `pending_results`
    /// 3. 发送消息到插件
    /// 4. 如果发送失败，清理已注册的 channel 并返回错误
    /// 5. 等待 receiver 收到 `ResultMessage`
    ///
    /// # 注意
    ///
    /// 如果 peer 在读循环中关闭，所有 pending 的 oneshot sender 会被
    /// 发送一个失败的结果，因此 `receiver.await` 会返回 `Err`（channel 关闭）。
    async fn await_result<T>(&self, request: T) -> Result<ResultMessage>
    where
        T: Into<InvokeOrInitialize>,
    {
        let request = request.into();
        let request_id = request.id().to_string();
        let (sender, receiver) = oneshot::channel();
        self.inner
            .pending_results
            .lock()
            .await
            .insert(request_id.clone(), sender);
        if let Err(error) = self.send_message(&request.into_message()).await {
            self.inner.pending_results.lock().await.remove(&request_id);
            return Err(error);
        }
        receiver.await.map_err(|_| {
            AstrError::Internal(format!(
                "peer dropped pending result channel '{}'",
                request_id
            ))
        })
    }

    /// 通过底层传输发送一条 JSON-RPC 消息。
    async fn send_message(&self, message: &PluginMessage) -> Result<()> {
        self.inner.send_message(message).await
    }
}

/// 统一封装 `InitializeMessage` 和 `InvokeMessage`。
///
/// 两种消息都遵循请求-响应模式：发送后等待 `ResultMessage`。
/// 此枚举允许 `await_result` 泛型处理两种请求类型，避免代码重复。
enum InvokeOrInitialize {
    Initialize(InitializeMessage),
    Invoke(InvokeMessage),
}

impl InvokeOrInitialize {
    fn id(&self) -> &str {
        match self {
            Self::Initialize(message) => &message.id,
            Self::Invoke(message) => &message.id,
        }
    }

    fn into_message(self) -> PluginMessage {
        match self {
            Self::Initialize(message) => PluginMessage::Initialize(message),
            Self::Invoke(message) => PluginMessage::Invoke(message),
        }
    }
}

impl From<InitializeMessage> for InvokeOrInitialize {
    fn from(value: InitializeMessage) -> Self {
        Self::Initialize(value)
    }
}

impl From<InvokeMessage> for InvokeOrInitialize {
    fn from(value: InvokeMessage) -> Self {
        Self::Invoke(value)
    }
}

impl PeerInner {
    /// 后台读循环——持续监听来自插件的入站消息。
    ///
    /// # 退出条件
    ///
    /// - 传输层返回 `None`（管道关闭）→ 标记 "transport closed"
    /// - 传输层返回错误 → 标记错误信息
    /// - JSON 反序列化失败 → 标记 "failed to decode plugin message"
    /// - 消息处理器返回错误 → 标记 "peer message handling failed"
    ///
    /// 任何退出都会调用 `close()` 通知所有等待方。
    async fn read_loop(self: Arc<Self>) {
        loop {
            match self.transport.recv().await {
                Ok(Some(payload)) => {
                    let message = match serde_json::from_str::<PluginMessage>(&payload) {
                        Ok(message) => message,
                        Err(error) => {
                            self.close(format!("failed to decode plugin message: {error}"))
                                .await;
                            break;
                        },
                    };
                    if let Err(error) = Arc::clone(&self).handle_message(message).await {
                        self.close(format!("peer message handling failed: {error}"))
                            .await;
                        break;
                    }
                },
                Ok(None) => {
                    self.close("transport closed".to_string()).await;
                    break;
                },
                Err(error) => {
                    self.close(error).await;
                    break;
                },
            }
        }
    }

    /// 分发入站消息到对应的处理器。
    ///
    /// # 注意
    ///
    /// `Invoke` 消息的处理器返回 `Ok(())` 因为实际的异步处理在
    /// 独立的 tokio task 中进行，这里只负责 spawn 并立即返回。
    /// 如果 spawn 或后续处理失败，会在 task 内部调用 `close()`。
    async fn handle_message(self: Arc<Self>, message: PluginMessage) -> Result<()> {
        match message {
            PluginMessage::Initialize(message) => self.handle_initialize(message).await,
            PluginMessage::Invoke(message) => {
                self.handle_invoke(message).await;
                Ok(())
            },
            PluginMessage::Result(message) => self.handle_result(message).await,
            PluginMessage::Event(message) => self.handle_event(message).await,
            PluginMessage::Cancel(message) => self.handle_cancel(message).await,
        }
    }

    /// 处理插件发起的初始化请求（插件→宿主的反向握手）。
    ///
    /// 验证协议版本兼容性：如果插件声明的版本中包含 `PROTOCOL_VERSION`，
    /// 则协商成功并返回本地能力信息；否则返回版本不匹配错误。
    ///
    /// # 成功响应
    ///
    /// 包含本地声明的能力、handlers、profiles 和元数据。
    ///
    /// # 失败响应
    ///
    /// 错误码为 `unsupported_version`，`retriable: false` 表示不应重试。
    async fn handle_initialize(&self, message: InitializeMessage) -> Result<()> {
        let InitializeMessage {
            id,
            protocol_version,
            supported_protocol_versions,
            peer,
            capabilities,
            handlers,
            profiles,
            metadata,
        } = message;
        let supported = protocol_version == PROTOCOL_VERSION
            || supported_protocol_versions
                .iter()
                .any(|version| version == PROTOCOL_VERSION);
        let response = if supported {
            let negotiated = InitializeResultData {
                protocol_version: PROTOCOL_VERSION.to_string(),
                peer,
                capabilities,
                handlers,
                profiles,
                skills: vec![],
                modes: vec![],
                metadata,
            };
            *self.remote_initialize.lock().await = Some(negotiated.clone());
            ResultMessage {
                id,
                kind: Some("initialize".to_string()),
                success: true,
                output: serde_json::to_value(self.local_result()).map_err(|error| {
                    AstrError::Validation(format!(
                        "failed to serialize local initialize result: {error}"
                    ))
                })?,
                error: None,
                metadata: json!({ "acceptedVersion": PROTOCOL_VERSION }),
            }
        } else {
            ResultMessage {
                id,
                kind: Some("initialize".to_string()),
                success: false,
                output: Value::Null,
                error: Some(ErrorPayload {
                    code: "unsupported_version".to_string(),
                    message: format!(
                        "peer version '{}' does not support '{}'",
                        protocol_version, PROTOCOL_VERSION
                    ),
                    details: json!({ "supportedProtocolVersions": supported_protocol_versions }),
                    retriable: false,
                }),
                metadata: Value::Null,
            }
        };
        self.send_message(&PluginMessage::Result(response)).await
    }

    /// 处理插件发起的能力调用请求（插件→宿主）。
    ///
    /// 此方法在独立的 tokio task 中执行，不阻塞读循环。
    ///
    /// # 生命周期管理
    ///
    /// 1. 创建 `CancelToken` 并注册到 `inbound_cancellations`
    /// 2. 根据 `stream` 标志选择流式或一元处理
    /// 3. 完成后清理取消令牌和 invoke handle
    /// 4. 如果处理失败，关闭整个 peer（因为插件可能处于不一致状态）
    ///
    /// # 为什么失败时关闭 peer？
    ///
    /// 入站 invoke 是插件主动调用宿主能力，如果处理失败通常意味着
    /// 宿主侧出现了严重问题（如能力未注册、权限拒绝等），
    /// 继续运行可能导致更严重的不一致。
    async fn handle_invoke(self: Arc<Self>, message: InvokeMessage) {
        let request_id = message.id.clone();
        let track_id = request_id.clone();
        let handle = tokio::spawn({
            let this = Arc::clone(&self);
            async move {
                let cancel = CancelToken::new();
                this.inbound_cancellations
                    .lock()
                    .await
                    .insert(request_id.clone(), cancel.clone());

                let result = if message.stream {
                    this.handle_streaming_invoke(message, cancel).await
                } else {
                    this.handle_unary_invoke(message, cancel).await
                };

                this.inbound_cancellations.lock().await.remove(&request_id);
                this.invoke_handles
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&request_id);
                if let Err(error) = result {
                    this.close(format!("failed to process inbound invoke: {error}"))
                        .await;
                }
            }
        });

        // Track the handle so shutdown cannot miss an in-flight invoke.
        self.invoke_handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(track_id, handle);
    }

    /// 处理一元（非流式）入站调用。
    ///
    /// 通过 `CapabilityRouter` 调用本地能力，将结果封装为 `ResultMessage`
    /// 发送回插件。流式调用使用 `EventEmitter::noop()` 因为一元调用不需要增量输出。
    async fn handle_unary_invoke(&self, message: InvokeMessage, cancel: CancelToken) -> Result<()> {
        let result = self
            .router
            .invoke(
                &message.capability,
                message.input,
                message.context,
                EventEmitter::noop(),
                cancel,
            )
            .await;
        let response = match result {
            Ok(output) => ResultMessage::success(message.id, output),
            Err(error) => ResultMessage::failure(message.id, error_to_payload(&error)),
        };
        self.send_message(&PluginMessage::Result(response)).await
    }

    /// 处理流式入站调用。
    ///
    /// 与一元调用不同，流式调用通过 `EventEmitter` 将增量事件
    /// 实时发送回插件。
    ///
    /// # 流程
    ///
    /// 1. 发送 `Started` 事件，告知插件调用已开始
    /// 2. 创建 `EventEmitter`，每个 `delta()` 调用都会通过传输层发送 `EventMessage`
    /// 3. 调用 `CapabilityRouter` 执行实际能力
    /// 4. 根据结果发送 `Completed` 或 `Failed` 终端事件
    ///
    /// # 取消处理
    ///
    /// 如果能力执行成功但 `cancel.is_cancelled()` 为 true，
    /// 发送 `Failed` 事件（错误码 `cancelled`），因为结果可能不完整。
    async fn handle_streaming_invoke(
        self: &Arc<Self>,
        message: InvokeMessage,
        cancel: CancelToken,
    ) -> Result<()> {
        let request_id = message.id.clone();
        let sequence = Arc::new(AtomicU64::new(1));
        self.send_message(&PluginMessage::Event(EventMessage {
            id: request_id.clone(),
            phase: EventPhase::Started,
            event: "invoke.started".to_string(),
            payload: json!({ "capability": message.capability }),
            seq: 0,
            error: None,
        }))
        .await?;

        let transport = Arc::clone(&self.transport);
        let emit_request_id = request_id.clone();
        let emit_sequence = Arc::clone(&sequence);
        let emitter = EventEmitter::new(move |event, payload| {
            let transport = Arc::clone(&transport);
            let emit_request_id = emit_request_id.clone();
            let emit_sequence = Arc::clone(&emit_sequence);
            async move {
                let event_message = PluginMessage::Event(EventMessage {
                    id: emit_request_id,
                    phase: EventPhase::Delta,
                    event,
                    payload,
                    seq: emit_sequence.fetch_add(1, Ordering::SeqCst),
                    error: None,
                });
                send_message_via_transport(transport, &event_message).await
            }
        });

        let result = self
            .router
            .invoke(
                &message.capability,
                message.input,
                message.context,
                emitter,
                cancel.clone(),
            )
            .await;

        let terminal = match result {
            Ok(_output) if cancel.is_cancelled() => EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "invoke.cancelled".to_string(),
                payload: Value::Null,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: Some(ErrorPayload {
                    code: "cancelled".to_string(),
                    message: "request was cancelled".to_string(),
                    details: Value::Null,
                    retriable: false,
                }),
            },
            Ok(output) => EventMessage {
                id: request_id,
                phase: EventPhase::Completed,
                event: "invoke.completed".to_string(),
                payload: output,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: None,
            },
            Err(error) => EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "invoke.failed".to_string(),
                payload: Value::Null,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: Some(error_to_payload(&error)),
            },
        };

        self.send_message(&PluginMessage::Event(terminal)).await
    }

    /// 处理插件返回的结果消息。
    ///
    /// 从 `pending_results` 中取出对应的 oneshot sender 并发送结果。
    /// 如果没有匹配的 sender（可能是重复消息或已超时），则静默忽略。
    async fn handle_result(&self, message: ResultMessage) -> Result<()> {
        if let Some(sender) = self.pending_results.lock().await.remove(&message.id) {
            // 故意忽略：接收端已关闭表示请求已被取消/超时
            let _ = sender.send(message);
        }
        Ok(())
    }

    /// 处理插件发送的事件消息。
    ///
    /// 将事件转发到 `pending_streams` 中对应的 channel。
    /// 如果是终端事件（Completed/Failed），处理完后从 map 中移除，
    /// 避免后续消息丢失时 channel 泄漏。
    async fn handle_event(&self, message: EventMessage) -> Result<()> {
        let is_terminal = matches!(message.phase, EventPhase::Completed | EventPhase::Failed);
        let request_id = message.id.clone();
        let mut streams = self.pending_streams.lock().await;
        if let Some(sender) = streams.get(&request_id) {
            // 故意忽略：流已关闭表示订阅者已离开
            let _ = sender.send(message);
        }
        if is_terminal {
            streams.remove(&request_id);
        }
        Ok(())
    }

    /// 处理插件发送的取消请求。
    ///
    /// 从 `inbound_cancellations` 中取出对应的 `CancelToken` 并触发取消。
    /// 如果没有匹配的 token（可能已处理完或从未注册），则静默忽略。
    async fn handle_cancel(&self, message: CancelMessage) -> Result<()> {
        if let Some(cancel) = self
            .inbound_cancellations
            .lock()
            .await
            .get(&message.id)
            .cloned()
        {
            cancel.cancel();
        }
        Ok(())
    }

    /// 将本地初始化信息转换为 `InitializeResultData`。
    ///
    /// 用于响应插件的 `InitializeMessage`，告知插件本端支持的能力。
    fn local_result(&self) -> InitializeResultData {
        InitializeResultData {
            protocol_version: self.local_initialize.protocol_version.clone(),
            peer: self.local_initialize.peer.clone(),
            capabilities: self.local_initialize.capabilities.clone(),
            handlers: self.local_initialize.handlers.clone(),
            profiles: self.local_initialize.profiles.clone(),
            skills: vec![],
            modes: vec![],
            metadata: self.local_initialize.metadata.clone(),
        }
    }

    /// 通过底层传输发送一条 JSON-RPC 消息。
    ///
    /// 将 `PluginMessage` 序列化为 JSON 字符串后发送。
    async fn send_message(&self, message: &PluginMessage) -> Result<()> {
        send_message_via_transport(Arc::clone(&self.transport), message).await
    }

    /// 关闭 Peer 并通知所有等待方。
    ///
    /// 这是一个幂等操作：如果已经关闭过，则直接返回。
    ///
    /// # 清理流程
    ///
    /// 1. 设置 `closed_reason`（防止重复关闭）
    /// 2. 向所有 `pending_results` 发送失败结果
    /// 3. 向所有 `pending_streams` 发送 failed 终端事件
    /// 4. 唤醒所有等待 `closed_notify` 的异步任务
    async fn close(&self, reason: String) {
        let mut closed_reason = self.closed_reason.lock().await;
        if closed_reason.is_some() {
            return;
        }
        *closed_reason = Some(reason.clone());
        drop(closed_reason);

        let error = ErrorPayload {
            code: "transport_closed".to_string(),
            message: reason.clone(),
            details: Value::Null,
            retriable: false,
        };

        let pending_results = std::mem::take(&mut *self.pending_results.lock().await);
        for (request_id, sender) in pending_results {
            // 故意忽略：发送失败响应时接收端可能已关闭
            let _ = sender.send(ResultMessage::failure(request_id, error.clone()));
        }

        let pending_streams = std::mem::take(&mut *self.pending_streams.lock().await);
        for (request_id, sender) in pending_streams {
            // 故意忽略：广播关闭事件时接收端可能已关闭
            let _ = sender.send(EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "transport.closed".to_string(),
                payload: Value::Null,
                seq: 0,
                error: Some(error.clone()),
            });
        }

        self.closed_notify.notify_waiters();
    }
}

/// 将 JSON-RPC 消息序列化并通过传输层发送。
///
/// # 错误处理
///
/// - 序列化失败返回 `Validation` 错误（通常是消息结构问题）
/// - 发送失败返回 `Internal` 错误（通常是传输层问题，如管道断裂）
async fn send_message_via_transport(
    transport: Arc<dyn Transport>,
    message: &PluginMessage,
) -> Result<()> {
    let payload = serde_json::to_string(message).map_err(|error| {
        AstrError::Validation(format!("failed to serialize plugin message: {error}"))
    })?;
    transport
        .send(&payload)
        .await
        .map_err(|error| AstrError::Internal(format!("failed to send plugin message: {error}")))
}

/// 将 `AstrError` 转换为 JSON-RPC 错误载荷。
///
/// 根据错误类型映射到对应的错误码：
/// - `Cancelled` / `LlmInterrupted` → `cancelled`
/// - `Validation` → `validation_error`
/// - `Io` → `io_error`
/// - `Parse` → `parse_error`
/// - 其他 → `internal_error`
///
/// `retriable` 字段继承自 `AstrError::is_retryable()`，
/// 告知调用方是否应该重试。
fn error_to_payload(error: &AstrError) -> ErrorPayload {
    ErrorPayload {
        code: match error {
            AstrError::Cancelled | AstrError::LlmInterrupted => "cancelled",
            AstrError::Validation(_) => "validation_error",
            AstrError::Io { .. } => "io_error",
            AstrError::Parse { .. } => "parse_error",
            _ => "internal_error",
        }
        .to_string(),
        message: error.to_string(),
        details: Value::Null,
        retriable: error.is_retryable(),
    }
}

/// 将 `ResultMessage` 的错误载荷转换回 `AstrError`。
///
/// 特殊处理 `cancelled` 错误码，将其映射为 `AstrError::Cancelled`，
/// 其他错误统一映射为 `Internal` 并保留原始错误信息。
fn result_error_to_astr(result: ResultMessage) -> AstrError {
    let request_id = result.id;
    match result.error {
        Some(error) if error.code == "cancelled" => AstrError::Cancelled,
        Some(error) => AstrError::Internal(format!(
            "plugin request '{}' failed: {}",
            request_id, error.message
        )),
        None => AstrError::Internal(format!(
            "plugin request '{}' failed without error payload",
            request_id
        )),
    }
}
