//! server 私有的 capability router。
//!
//! Why: `6.5.5b` 需要删除 `server -> astrcode-kernel` 的正式依赖，
//! 运行时端口仍需要一份最小的 in-memory router 来承接 turn 执行和测试夹具。
//! 这里把仍被使用的最小路由逻辑收敛到 server，避免继续把整个
//! `astrcode-kernel` crate 留在活跃依赖图里。

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use astrcode_core::{
    AstrError, CapabilityInvoker, CapabilitySpec, Result, ToolCallRequest, ToolContext,
    ToolExecutionResult, support,
};

fn validate_capability_spec(capability_spec: &CapabilitySpec) -> Result<()> {
    capability_spec.validate().map_err(|error| {
        AstrError::Validation(format!(
            "invalid capability spec '{}': {}",
            capability_spec.name, error
        ))
    })
}

fn build_registry_snapshot(
    invokers: impl IntoIterator<Item = Arc<dyn CapabilityInvoker>>,
) -> Result<CapabilityRouterInner> {
    let mut invokers_by_name = HashMap::new();
    let mut order = Vec::new();

    for invoker in invokers {
        let capability_spec = invoker.capability_spec();
        validate_capability_spec(&capability_spec)?;
        if invokers_by_name
            .insert(capability_spec.name.to_string(), Arc::clone(&invoker))
            .is_some()
        {
            return Err(AstrError::Validation(format!(
                "duplicate capability '{}' registered",
                capability_spec.name
            )));
        }
        order.push(capability_spec.name.to_string());
    }

    Ok(CapabilityRouterInner {
        invokers_by_name,
        order,
    })
}

pub(crate) struct CapabilityRouterBuilder {
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl Default for CapabilityRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityRouterBuilder {
    pub(crate) fn new() -> Self {
        Self {
            invokers: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn register_invoker(mut self, invoker: Arc<dyn CapabilityInvoker>) -> Self {
        self.invokers.push(invoker);
        self
    }

    pub(crate) fn build(self) -> Result<CapabilityRouter> {
        let snapshot = build_registry_snapshot(self.invokers)?;
        Ok(CapabilityRouter {
            inner: Arc::new(RwLock::new(snapshot)),
        })
    }
}

struct CapabilityRouterInner {
    invokers_by_name: HashMap<String, Arc<dyn CapabilityInvoker>>,
    order: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct CapabilityRouter {
    inner: Arc<RwLock<CapabilityRouterInner>>,
}

impl Default for CapabilityRouter {
    fn default() -> Self {
        Self::empty()
    }
}

impl CapabilityRouter {
    pub(crate) fn builder() -> CapabilityRouterBuilder {
        CapabilityRouterBuilder::new()
    }

    pub(crate) fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CapabilityRouterInner {
                invokers_by_name: HashMap::new(),
                order: Vec::new(),
            })),
        }
    }

    pub(crate) fn replace_invokers(&self, invokers: Vec<Arc<dyn CapabilityInvoker>>) -> Result<()> {
        let snapshot = build_registry_snapshot(invokers)?;
        support::with_write_lock_recovery(&self.inner, "capability_router", |inner| {
            *inner = snapshot;
            Ok(())
        })
    }

    pub(crate) fn capability_specs(&self) -> Vec<CapabilitySpec> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner
                .order
                .iter()
                .filter_map(|name| inner.invokers_by_name.get(name))
                .map(|invoker| invoker.capability_spec())
                .collect()
        })
    }

    pub(crate) fn capability_spec(&self, name: &str) -> Option<CapabilitySpec> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner
                .invokers_by_name
                .get(name)
                .map(|invoker| invoker.capability_spec())
        })
    }

    pub(crate) async fn execute_tool(
        &self,
        call: &ToolCallRequest,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let invoker = support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner.invokers_by_name.get(&call.name).cloned()
        });

        let Some(invoker) = invoker else {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("unknown tool '{}'", call.name)),
                metadata: None,
                continuation: None,
                duration_ms: 0,
                truncated: false,
            };
        };

        let capability_spec = invoker.capability_spec();
        if !capability_spec.kind.is_tool() {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("capability '{}' is not tool-callable", call.name)),
                metadata: None,
                continuation: None,
                duration_ms: 0,
                truncated: false,
            };
        }

        let capability_ctx = crate::tool_capability_invoker::capability_context_from_tool_context(
            ctx,
            Some(call.id.clone()),
        );

        match invoker.invoke(call.args.clone(), &capability_ctx).await {
            Ok(result) => result.into_tool_execution_result(call.id.clone()),
            Err(error) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: None,
                continuation: None,
                duration_ms: 0,
                truncated: false,
            },
        }
    }
}
