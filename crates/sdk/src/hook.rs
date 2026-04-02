//! Plugin-local policy utilities.
//!
//! These hooks run inside a plugin process and help plugin authors compose reusable allow/deny
//! checks around their own handlers. They are intentionally narrower than the host runtime's
//! global policy contract, which also covers approval, context pressure, and model request
//! rewriting.

use std::sync::Arc;

use astrcode_protocol::plugin::CapabilityDescriptor;
use serde_json::Value;

use crate::{PluginContext, SdkError};

/// 策略决策结果。
///
/// 由 `PolicyHook::before_invoke` 返回，
/// 决定工具是否被允许执行。
///
/// ## 默认语义
///
/// 多个钩子串联时，最后一个钩子的决策为最终结果（除非配置了短路）。
/// 短路模式下，第一个 deny 会立即终止后续钩子执行。
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDecision {
    /// 是否允许执行。
    ///
    /// `true` 表示放行，`false` 表示拒绝。
    pub allowed: bool,
    /// 拒绝原因（仅在 `allowed = false` 时有意义）。
    pub reason: Option<String>,
    /// 附加的元数据，可包含结构化信息供前端展示或日志记录。
    pub metadata: Value,
}

impl PolicyDecision {
    /// 构造允许的决策。
    ///
    /// 默认不带原因和元数据，钩子可在后续链中附加信息。
    pub fn allow() -> Self {
        Self {
            allowed: true,
            reason: None,
            metadata: Value::Null,
        }
    }

    /// 构造拒绝的决策。
    ///
    /// `reason` 会展示给用户或记录到日志中，
    /// 应清晰说明拒绝的具体原因。
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: Some(reason.into()),
            metadata: Value::Null,
        }
    }
}

/// A lightweight pre-invocation guard around plugin-owned capabilities.
///
/// Use this for plugin-local validation and gating. Host-level approval, sandbox, or runtime
/// policy should stay in the host runtime rather than being reimplemented here.
pub trait PolicyHook: Send + Sync {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        context: &PluginContext,
    ) -> PolicyDecision;
}

/// 钩子短路策略。
///
/// 控制 `PolicyHookChain` 在遇到 deny 决策时的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookShortCircuit {
    /// 永不短路，始终执行所有钩子。
    ///
    /// 适用于需要收集所有钩子诊断信息的场景。
    Never,
    /// 遇到第一个 deny 时立即停止。
    ///
    /// 默认行为，因为策略检查通常采用"一票否决"原则，
    /// 一旦有钩子拒绝，后续检查无意义且浪费资源。
    #[default]
    OnDeny,
}

/// 已注册的策略钩子包装器。
///
/// 将钩子名称与实现绑定，使用 `Arc` 包装以支持
/// 在 `PolicyHookChain` 中共享而无需克隆实现。
#[derive(Clone)]
pub struct RegisteredPolicyHook {
    name: String,
    hook: Arc<dyn PolicyHook>,
}

impl RegisteredPolicyHook {
    /// 返回钩子名称。
    ///
    /// 用于日志记录和错误消息中标识是哪个钩子做出的决策。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 返回钩子实现的引用。
    pub fn hook(&self) -> &dyn PolicyHook {
        self.hook.as_ref()
    }
}

/// 策略钩子注册表。
///
/// 管理插件内注册的所有策略钩子，
/// 提供 builder 风格（`with_policy_hook`）和可变风格（`register_policy_hook`）两种 API。
///
/// ## 为什么需要注册表
///
/// 插件可能有多个工具需要共享同一组策略检查（如路径白名单、
/// 权限验证）。注册表允许一次注册、多处复用，
/// 并通过 `policy_hook_chain()` 快速构建执行链。
#[derive(Clone, Default)]
pub struct HookRegistry {
    policy_hooks: Vec<RegisteredPolicyHook>,
}

impl HookRegistry {
    /// 注册一个策略钩子。
    ///
    /// ## 参数
    ///
    /// - `name`: 钩子名称，用于日志和错误消息标识
    /// - `hook`: 策略钩子实现
    ///
    /// ## 错误
    ///
    /// 如果名称已存在或为空，返回 `SdkError::Validation`。
    pub fn register_policy_hook<H>(
        &mut self,
        name: impl Into<String>,
        hook: H,
    ) -> Result<&mut Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        let name = normalize_hook_name(name)?;
        if self
            .policy_hooks
            .iter()
            .any(|registered| registered.name == name)
        {
            return Err(SdkError::validation(format!(
                "duplicate policy hook registration '{name}'"
            )));
        }
        self.policy_hooks.push(RegisteredPolicyHook {
            name,
            hook: Arc::new(hook),
        });
        Ok(self)
    }

    /// 以 builder 风格注册策略钩子。
    ///
    /// 与 `register_policy_hook` 功能相同，
    /// 但返回 `Self` 而非 `&mut Self`，支持链式调用。
    pub fn with_policy_hook<H>(mut self, name: impl Into<String>, hook: H) -> Result<Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        self.register_policy_hook(name, hook)?;
        Ok(self)
    }

    /// 返回所有已注册的策略钩子。
    pub fn policy_hooks(&self) -> &[RegisteredPolicyHook] {
        &self.policy_hooks
    }

    /// 从当前注册表构建策略钩子执行链。
    ///
    /// 返回的 `PolicyHookChain` 包含所有已注册钩子的克隆引用，
    /// 可独立配置短路策略而不影响原始注册表。
    pub fn policy_hook_chain(&self) -> PolicyHookChain {
        // Registry owns hooks by Arc so plugin authors can build reusable chains
        // without moving or reconstructing the original registrations.
        PolicyHookChain {
            hooks: self.policy_hooks.clone(),
            short_circuit: HookShortCircuit::default(),
        }
    }
}

/// 策略钩子执行链。
///
/// 将多个 `PolicyHook` 组合为一个可顺序执行的链，
/// 每个钩子依次对工具调用进行前置检查。
///
/// ## 执行语义
///
/// - 默认短路模式：第一个 deny 立即终止链，返回该 deny 决策
/// - 非短路模式：执行所有钩子，返回最后一个钩子的决策
///
/// `PolicyHookChain` 自身也实现 `PolicyHook`，因此可以嵌套组合。
#[derive(Clone, Default)]
pub struct PolicyHookChain {
    hooks: Vec<RegisteredPolicyHook>,
    short_circuit: HookShortCircuit,
}

impl PolicyHookChain {
    /// 配置短路策略。
    ///
    /// 返回新的 `PolicyHookChain`（builder 风格），
    /// 不影响原实例。
    pub fn with_short_circuit(mut self, short_circuit: HookShortCircuit) -> Self {
        self.short_circuit = short_circuit;
        self
    }

    /// 注册一个策略钩子到此链。
    ///
    /// 与 `HookRegistry::register_policy_hook` 类似，
    /// 但钩子仅加入此链，不影响注册表。
    pub fn register<H>(&mut self, name: impl Into<String>, hook: H) -> Result<&mut Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        let name = normalize_hook_name(name)?;
        if self.hooks.iter().any(|registered| registered.name == name) {
            return Err(SdkError::validation(format!(
                "duplicate policy hook registration '{name}'"
            )));
        }
        self.hooks.push(RegisteredPolicyHook {
            name,
            hook: Arc::new(hook),
        });
        Ok(self)
    }

    /// 以 builder 风格注册钩子。
    pub fn with_hook<H>(mut self, name: impl Into<String>, hook: H) -> Result<Self, SdkError>
    where
        H: PolicyHook + 'static,
    {
        self.register(name, hook)?;
        Ok(self)
    }

    /// 返回链中所有已注册的钩子。
    pub fn hooks(&self) -> &[RegisteredPolicyHook] {
        &self.hooks
    }

    /// 返回当前的短路策略。
    pub fn short_circuit(&self) -> HookShortCircuit {
        self.short_circuit
    }
}

impl PolicyHook for PolicyHookChain {
    fn before_invoke(
        &self,
        capability: &CapabilityDescriptor,
        context: &PluginContext,
    ) -> PolicyDecision {
        let mut final_decision = PolicyDecision::allow();
        for registered in &self.hooks {
            let decision = registered.hook.before_invoke(capability, context);
            // Policies default to fail-fast on deny so a guard hook can veto the
            // invocation before later hooks add more permissive behavior.
            let should_stop =
                matches!(self.short_circuit, HookShortCircuit::OnDeny) && !decision.allowed;
            final_decision = decision;
            if should_stop {
                break;
            }
        }
        final_decision
    }
}

fn normalize_hook_name(name: impl Into<String>) -> Result<String, SdkError> {
    let name = name.into();
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SdkError::validation(
            "policy hook registration requires a non-empty name",
        ));
    }
    Ok(trimmed.to_string())
}
