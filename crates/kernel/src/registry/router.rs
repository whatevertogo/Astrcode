//! 能力路由器具体实现。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use astrcode_core::{
    AstrError, CapabilityInvoker, CapabilitySpec, Result, ToolCallRequest, ToolContext,
    ToolDefinition, ToolExecutionResult,
    support::{self},
};

use super::tool::capability_context_from_tool_context;

pub struct CapabilityRouterBuilder {
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl Default for CapabilityRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityRouterBuilder {
    pub fn new() -> Self {
        Self {
            invokers: Vec::new(),
        }
    }

    pub fn register_invoker(mut self, invoker: Arc<dyn CapabilityInvoker>) -> Self {
        self.invokers.push(invoker);
        self
    }

    pub fn build(self) -> Result<CapabilityRouter> {
        let mut invokers_by_name = HashMap::new();
        let mut order = Vec::new();
        let mut tool_order = Vec::new();

        for invoker in self.invokers {
            let capability_spec = invoker.capability_spec();
            capability_spec.validate().map_err(|error| {
                AstrError::Validation(format!(
                    "invalid capability spec '{}': {}",
                    capability_spec.name, error
                ))
            })?;
            if invokers_by_name
                .insert(capability_spec.name.to_string(), Arc::clone(&invoker))
                .is_some()
            {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    capability_spec.name
                )));
            }
            if capability_spec.kind.is_tool() {
                tool_order.push(capability_spec.name.to_string());
            }
            order.push(capability_spec.name.to_string());
        }

        Ok(CapabilityRouter {
            inner: Arc::new(RwLock::new(CapabilityRouterInner {
                invokers_by_name,
                order,
                tool_order,
            })),
        })
    }
}

struct CapabilityRouterInner {
    invokers_by_name: HashMap<String, Arc<dyn CapabilityInvoker>>,
    order: Vec<String>,
    tool_order: Vec<String>,
}

#[derive(Clone)]
pub struct CapabilityRouter {
    inner: Arc<RwLock<CapabilityRouterInner>>,
}

impl Default for CapabilityRouter {
    fn default() -> Self {
        Self::empty()
    }
}

impl CapabilityRouter {
    pub fn builder() -> CapabilityRouterBuilder {
        CapabilityRouterBuilder::new()
    }

    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CapabilityRouterInner {
                invokers_by_name: HashMap::new(),
                order: Vec::new(),
                tool_order: Vec::new(),
            })),
        }
    }

    pub fn register_invoker(&self, invoker: Arc<dyn CapabilityInvoker>) -> Result<()> {
        let capability_spec = invoker.capability_spec();
        capability_spec.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid capability spec '{}': {}",
                capability_spec.name, error
            ))
        })?;

        support::with_write_lock_recovery(&self.inner, "capability_router", |inner| {
            if inner
                .invokers_by_name
                .contains_key(capability_spec.name.as_str())
            {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    capability_spec.name
                )));
            }

            if capability_spec.kind.is_tool() {
                inner.tool_order.push(capability_spec.name.to_string());
            }
            inner.order.push(capability_spec.name.to_string());
            inner
                .invokers_by_name
                .insert(capability_spec.name.to_string(), invoker);
            Ok(())
        })
    }

    pub fn register_invokers(&self, invokers: Vec<Arc<dyn CapabilityInvoker>>) -> Result<()> {
        support::with_write_lock_recovery(&self.inner, "capability_router", |inner| {
            for invoker in &invokers {
                let capability_spec = invoker.capability_spec();
                capability_spec.validate().map_err(|error| {
                    AstrError::Validation(format!(
                        "invalid capability spec '{}': {}",
                        capability_spec.name, error
                    ))
                })?;
                if inner
                    .invokers_by_name
                    .contains_key(capability_spec.name.as_str())
                {
                    return Err(AstrError::Validation(format!(
                        "duplicate capability '{}' registered",
                        capability_spec.name
                    )));
                }
            }

            for invoker in invokers {
                let capability_spec = invoker.capability_spec();
                if capability_spec.kind.is_tool() {
                    inner.tool_order.push(capability_spec.name.to_string());
                }
                inner.order.push(capability_spec.name.to_string());
                inner
                    .invokers_by_name
                    .insert(capability_spec.name.to_string(), invoker);
            }
            Ok(())
        })
    }

    pub fn capability_specs(&self) -> Vec<CapabilitySpec> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner
                .order
                .iter()
                .filter_map(|name| inner.invokers_by_name.get(name))
                .map(|invoker| invoker.capability_spec())
                .collect()
        })
    }

    pub fn descriptors(&self) -> Vec<CapabilitySpec> {
        self.capability_specs()
    }

    /// 返回按注册顺序排列的 invoker 快照。
    ///
    /// runtime surface 热替换需要拿到现有 invoker，再按来源增删外部能力后
    /// 重建整份路由；直接暴露 `Arc` 克隆可以避免重新解析 descriptor 丢失执行器。
    pub fn invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner
                .order
                .iter()
                .filter_map(|name| inner.invokers_by_name.get(name).cloned())
                .collect()
        })
    }

    pub fn capability_spec(&self, name: &str) -> Option<CapabilitySpec> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner
                .invokers_by_name
                .get(name)
                .map(|invoker| invoker.capability_spec())
        })
    }

    pub fn descriptor(&self, name: &str) -> Option<CapabilitySpec> {
        self.capability_spec(name)
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.capability_specs()
            .into_iter()
            .filter(|capability_spec| capability_spec.kind.is_tool())
            .map(|capability_spec| ToolDefinition {
                name: capability_spec.name.into_string(),
                description: capability_spec.description,
                parameters: capability_spec.input_schema,
            })
            .collect()
    }

    pub fn tool_names(&self) -> Vec<String> {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner.tool_order.clone()
        })
    }

    pub fn subset_for_tools(&self, allowed_tool_names: &[String]) -> Result<Self> {
        let allowed = allowed_tool_names
            .iter()
            .map(|name| name.as_str())
            .collect::<HashSet<_>>();
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            let mut builder = CapabilityRouter::builder();

            for name in &inner.order {
                let Some(invoker) = inner.invokers_by_name.get(name) else {
                    continue;
                };
                let capability_spec = invoker.capability_spec();
                if capability_spec.kind.is_tool()
                    && !allowed.contains(capability_spec.name.as_str())
                {
                    continue;
                }
                builder = builder.register_invoker(Arc::clone(invoker));
            }

            builder.build()
        })
    }

    pub async fn execute_tool(
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
                duration_ms: 0,
                truncated: false,
            };
        }

        let capability_ctx = capability_context_from_tool_context(ctx, Some(call.id.clone()));

        match invoker.invoke(call.args.clone(), &capability_ctx).await {
            Ok(result) => result.into_tool_execution_result(call.id.clone()),
            Err(error) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            },
        }
    }

    pub fn has_capability(&self, name: &str) -> bool {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner.invokers_by_name.contains_key(name)
        })
    }

    pub fn capability_count(&self) -> usize {
        support::with_read_lock_recovery(&self.inner, "capability_router", |inner| {
            inner.invokers_by_name.len()
        })
    }
}
