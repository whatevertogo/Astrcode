//! # 取消令牌
//!
//! 提供轻量级的跨线程取消信号机制，用于中断长时间运行的操作（如 LLM 请求、工具执行）。
//!
//! ## 设计动机
//!
//! 使用 `Arc<AtomicBool>` 而非 `tokio::CancellationToken`，是为了保持 core crate
//! 不依赖 tokio。core 只定义接口，运行时依赖由上层 crate 注入。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// 跨线程共享的取消信号。
///
/// `Clone` 实现是浅拷贝，所有副本共享同一个底层原子布尔值。
/// 任何副本调用 [`cancel()`](Self::cancel) 后，所有副本的
/// [`is_cancelled()`](Self::is_cancelled) 都会返回 `true`。
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// 创建一个新的未取消令牌。
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// 发出取消信号。
    ///
    /// 使用 `SeqCst` 排序确保所有线程都能立即看到取消状态。
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// 检查是否已被取消。
    ///
    /// 长时间运行的操作应在循环或关键检查点调用此方法，
    /// 以便及时响应取消请求。
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
