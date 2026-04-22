//! # Agent 生命周期与轮次结果
//!
//! 将旧 `AgentStatus` 拆分为两层：
//! - `AgentLifecycleStatus`：agent 的长期生命周期（Pending → Running → Idle → Terminated）
//! - `AgentTurnOutcome`：最近一轮执行的结束原因
//!
//! 拆分理由：旧 `AgentStatus` 同时承担生命周期（Pending/Running）和单轮结果（Completed/Failed），
//! 无法表达"agent 完成一轮后进入空闲可继续接收指令"这一四工具模型核心状态。

use serde::{Deserialize, Serialize};

/// Agent 的持久生命周期状态。
///
/// 与旧 `AgentStatus` 不同，该枚举只描述 agent 实例的长期存活阶段，
/// 不包含单轮执行的具体结束原因（后者由 `AgentTurnOutcome` 表达）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentLifecycleStatus {
    /// 已注册但尚未开始首轮执行。
    Pending,
    /// 正在执行某一轮 turn。
    Running,
    /// 单轮执行完成，等待新的 send 触发下一轮。
    /// 四工具模型核心状态：agent 完成一轮后不自动终止，而是进入 Idle。
    Idle,
    /// 已被 close 终止，不可恢复。
    Terminated,
}

impl AgentLifecycleStatus {
    /// 判断是否已经到达终态（不可恢复的已死状态）。
    pub fn is_final(self) -> bool {
        matches!(self, Self::Terminated)
    }

    /// 是否正在占用并发槽位。Pending/Running 占槽，Idle/Terminated 释放槽。
    pub fn occupies_slot(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    /// 判断 agent 当前是否可接收新消息。
    /// Pending 和 Idle 状态下可以立即触发下一轮；Running 状态下消息排队。
    pub fn can_accept_message(self) -> bool {
        matches!(self, Self::Pending | Self::Idle | Self::Running)
    }

    /// 判断 agent 当前是否空闲，可以被新消息立即唤醒。
    pub fn is_idle_or_pending(self) -> bool {
        matches!(self, Self::Pending | Self::Idle)
    }
}

/// Agent 单轮执行的结束原因。
///
/// 该枚举与 `AgentLifecycleStatus` 正交：
/// agent 完成一轮后（outcome 变为 Some），lifecycle 从 Running → Idle，
/// 而不是直接进入终态。只有在 close 被调用时才进入 Terminated。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentTurnOutcome {
    /// 正常完成本轮。
    Completed,
    /// 被取消。
    Cancelled,
    /// 因错误结束。
    Failed,
}

impl AgentTurnOutcome {
    /// 判断该 outcome 是否属于"异常结束"（可用于 UI 高亮或日志告警）。
    pub fn is_error(self) -> bool {
        matches!(self, Self::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_transitions_reflect_four_tool_model() {
        // Pending -> Running -> Idle -> Terminated 的核心流转
        assert!(AgentLifecycleStatus::Pending.can_accept_message());
        assert!(AgentLifecycleStatus::Running.can_accept_message());
        assert!(AgentLifecycleStatus::Idle.can_accept_message());
        assert!(!AgentLifecycleStatus::Terminated.can_accept_message());

        assert!(AgentLifecycleStatus::Idle.is_idle_or_pending());
        assert!(AgentLifecycleStatus::Pending.is_idle_or_pending());
        assert!(!AgentLifecycleStatus::Running.is_idle_or_pending());
    }

    #[test]
    fn turn_outcome_error_detection() {
        assert!(!AgentTurnOutcome::Completed.is_error());
        assert!(!AgentTurnOutcome::Cancelled.is_error());
        assert!(AgentTurnOutcome::Failed.is_error());
    }
}
