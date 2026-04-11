//! execution owner handle 与内部执行辅助逻辑。
//!
//! 模块职责划分：
//! - `root`：根执行入口（execute_root_agent）与 handle 类型定义
//! - `subagent`：作为工具执行子 agent
//! - `surface`：读取当前 runtime surface 并构造 scoped execution 输入
//! - `status`：sub-run 状态查询
//! - `cancel`：sub-run 取消控制
//! - `context`：bootstrap 阶段的延迟执行器桥与错误转换工具

mod cancel;
mod collaboration;
mod context;
pub(super) mod root;
mod status;
mod subagent;
mod surface;

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentLifecycleStatus, AgentMailboxEnvelope, AgentProfile, AgentProfileCatalog,
    AgentTurnOutcome, AstrError, EventTranslator, ExecutionOrchestrationBoundary,
    LiveSubRunControlBoundary, MailboxBatchAckedPayload, MailboxBatchStartedPayload,
    MailboxQueuedPayload, Result, SessionTurnAcquireResult, SpawnAgentParams, SubRunHandle,
    SubRunResult, ToolContext,
};
use astrcode_runtime_agent_control::PendingParentDelivery;
use astrcode_runtime_agent_tool::SubAgentExecutor;
use astrcode_runtime_execution::{ChildLifecycleStage, DeliveryBufferStage};
use astrcode_runtime_prompt::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptLayer,
};
use astrcode_runtime_session::{
    append_batch_acked, append_batch_started, append_mailbox_queued, prepare_session_execution,
};
use async_trait::async_trait;
pub(crate) use context::{
    DeferredCollaborationExecutor, DeferredSubAgentExecutor, service_error_to_astr,
};
pub use root::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};

use crate::service::{
    RuntimeService, ServiceError, ServiceResult,
    blocking_bridge::spawn_blocking_service,
    turn::{BudgetSettings, RuntimeTurnInput, complete_session_execution, run_session_turn},
};

impl RuntimeService {
    /// 获取 Agent 执行服务句柄。
    pub fn execution(self: &Arc<Self>) -> AgentExecutionServiceHandle {
        AgentExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    /// 获取 Tool 执行服务句柄。
    pub fn tools(self: &Arc<Self>) -> ToolExecutionServiceHandle {
        ToolExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }
}

#[async_trait]
impl SubAgentExecutor for AgentExecutionServiceHandle {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        self.launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl AgentProfileCatalog for AgentExecutionServiceHandle {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
}

/// 加载指定工作目录的 agent profile 注册表。
impl AgentExecutionServiceHandle {
    pub(super) async fn load_profiles_for_working_dir(
        &self,
        working_dir: &std::path::Path,
    ) -> ServiceResult<Arc<astrcode_runtime_agent_loader::AgentProfileRegistry>> {
        if let Some(cached) = self.runtime.scoped_agent_profiles.get(working_dir) {
            return Ok(Arc::clone(cached.value()));
        }

        let loader = self.runtime.agent_loader();
        let working_dir = working_dir.to_path_buf();
        let load_working_dir = working_dir.clone();
        let registry = crate::service::blocking_bridge::spawn_blocking_service(
            "load scoped agent profiles",
            move || {
                loader
                    .load_for_working_dir(Some(&load_working_dir))
                    .map_err(|error| {
                        ServiceError::Internal(astrcode_core::AstrError::Validation(
                            error.to_string(),
                        ))
                    })
            },
        )
        .await?;
        let registry = Arc::new(registry);

        if let Some(cached) = self.runtime.scoped_agent_profiles.get(&working_dir) {
            return Ok(Arc::clone(cached.value()));
        }

        self.runtime
            .scoped_agent_profiles
            .insert(working_dir, Arc::clone(&registry));
        Ok(registry)
    }
}

impl AgentExecutionServiceHandle {
    /// 查询指定 sub-run 的 live handle。
    pub async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<Option<SubRunHandle>> {
        let normalized_session_id = astrcode_runtime_session::normalize_session_id(session_id);
        Ok(self
            .runtime
            .agent_control
            .get(sub_run_id)
            .await
            .filter(|handle| {
                let handle_session_id =
                    astrcode_runtime_session::normalize_session_id(&handle.session_id);
                let child_session_id = handle
                    .child_session_id
                    .as_deref()
                    .map(astrcode_runtime_session::normalize_session_id);
                handle_session_id == normalized_session_id
                    || child_session_id.as_deref() == Some(normalized_session_id.as_str())
            }))
    }

    pub(super) async fn reactivate_parent_agent_if_idle(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) {
        let parent_session_id = astrcode_runtime_session::normalize_session_id(parent_session_id);
        let parent_session = match self.runtime.ensure_session_loaded(&parent_session_id).await {
            Ok(session) => session,
            Err(error) => {
                log::warn!(
                    "failed to load parent session for mailbox queue append: parentSession='{}', \
                     childAgent='{}', error='{}'",
                    parent_session_id,
                    notification.child_ref.agent_id,
                    error
                );
                return;
            },
        };
        if let Err(error) = self
            .append_parent_delivery_mailbox_queue(&parent_session, parent_turn_id, notification)
            .await
        {
            log::warn!(
                "failed to persist durable parent mailbox queue before wake: parentSession='{}', \
                 childAgent='{}', deliveryId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id,
                notification.notification_id,
                error
            );
            return;
        }

        let queued = self
            .runtime
            .agent_control
            .enqueue_parent_delivery(
                parent_session_id.clone(),
                parent_turn_id.to_string(),
                notification.clone(),
            )
            .await;
        if queued {
            self.runtime
                .observability
                .record_delivery_buffer(DeliveryBufferStage::Queued);
        }

        if let Err(error) = self
            .try_start_parent_delivery_turn(&parent_session_id)
            .await
        {
            self.runtime
                .observability
                .record_child_lifecycle(ChildLifecycleStage::ReactivationFailed);
            self.runtime
                .observability
                .record_delivery_buffer(DeliveryBufferStage::WakeFailed);
            log::warn!(
                "failed to schedule parent wake turn from child delivery: parentSession='{}', \
                 childAgent='{}', subRunId='{}', error='{}'",
                parent_session_id,
                notification.child_ref.agent_id,
                notification.child_ref.sub_run_id,
                error
            );
        }
    }

    pub(super) async fn try_start_parent_delivery_turn(
        &self,
        parent_session_id: &str,
    ) -> ServiceResult<bool> {
        let parent_session_id = astrcode_runtime_session::normalize_session_id(parent_session_id);
        let Some(delivery_batch) = self
            .runtime
            .agent_control
            .checkout_parent_delivery_batch(&parent_session_id)
            .await
        else {
            return Ok(false);
        };
        let batch_delivery_ids = delivery_batch
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>();
        let batch_id = uuid::Uuid::new_v4().to_string();
        let target_agent_id = delivery_batch
            .first()
            .and_then(|delivery| delivery.notification.child_ref.parent_agent_id.clone())
            .unwrap_or_default();

        self.runtime
            .observability
            .record_child_lifecycle(ChildLifecycleStage::ReactivationRequested);
        self.runtime
            .observability
            .record_delivery_buffer(DeliveryBufferStage::WakeRequested);

        let session = self
            .runtime
            .ensure_session_loaded(&parent_session_id)
            .await?;
        let runtime_config = { self.runtime.config.lock().await.runtime.clone() };
        let budget_settings = BudgetSettings {
            continuation_min_delta_tokens: crate::config::resolve_continuation_min_delta_tokens(
                &runtime_config,
            ),
            max_continuations: crate::config::resolve_max_continuations(&runtime_config),
        };
        let turn_id = uuid::Uuid::new_v4().to_string();
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let acquire_session_id = parent_session_id.clone();
        let acquire_turn_id = turn_id.clone();
        let acquire_result =
            spawn_blocking_service("acquire parent delivery wake turn lease", move || {
                session_manager
                    .try_acquire_turn(&acquire_session_id, &acquire_turn_id)
                    .map_err(ServiceError::from)
            })
            .await?;

        let turn_lease = match acquire_result {
            SessionTurnAcquireResult::Acquired(turn_lease) => turn_lease,
            SessionTurnAcquireResult::Busy(_) => {
                let _ = self
                    .runtime
                    .agent_control
                    .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                    .await;
                return Ok(false);
            },
        };

        let cancel = astrcode_core::CancelToken::new();
        if let Err(error) = prepare_session_execution(
            &session,
            &parent_session_id,
            &turn_id,
            cancel.clone(),
            turn_lease,
            None,
        ) {
            let _ = self
                .runtime
                .agent_control
                .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                .await;
            return Err(ServiceError::Internal(AstrError::Internal(
                error.to_string(),
            )));
        }

        let mut batch_translator = EventTranslator::new(session.current_phase()?);
        if let Err(error) = append_batch_started(
            &session,
            &turn_id,
            astrcode_core::AgentEventContext::default(),
            MailboxBatchStartedPayload {
                target_agent_id: target_agent_id.clone(),
                turn_id: turn_id.clone(),
                batch_id: batch_id.clone(),
                delivery_ids: batch_delivery_ids.clone(),
            },
            &mut batch_translator,
        )
        .await
        {
            let _ = self
                .runtime
                .agent_control
                .requeue_parent_delivery_batch(&parent_session_id, &batch_delivery_ids)
                .await;
            return Err(ServiceError::Internal(AstrError::Internal(format!(
                "failed to append parent delivery batch started: {error}"
            ))));
        }

        let loop_ = self.current_loop().await;
        let observability = self.runtime.observability.clone();
        let agent_control = self.control();
        let service = self.clone();
        let wake_session_id = parent_session_id.clone();
        let wake_turn_id = turn_id.clone();
        let wake_batch_id = batch_id.clone();
        let wake_target_agent_id = target_agent_id.clone();
        let wake_delivery_ids = batch_delivery_ids.clone();
        let runtime_input = RuntimeTurnInput {
            user_event: None,
            prompt_declarations: build_parent_delivery_prompt_declarations(&delivery_batch),
        };
        let handle = tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let result = run_session_turn(
                &session,
                &loop_,
                &wake_turn_id,
                cancel,
                runtime_input,
                astrcode_core::AgentEventContext::default(),
                astrcode_core::ExecutionOwner::root(
                    wake_session_id.clone(),
                    wake_turn_id.clone(),
                    astrcode_core::InvocationKind::RootExecution,
                ),
                budget_settings,
                Some(observability.clone()),
            )
            .await;

            complete_session_execution(&session, result.phase, &agent_control).await;

            let mut should_continue_draining = false;
            // 只有在 wake turn 成功后才消费 delivery，否则重新排队以防止子交付丢失
            if result.succeeded {
                let ack_result = async {
                    let mut translator = EventTranslator::new(
                        session
                            .current_phase()
                            .unwrap_or(astrcode_core::Phase::Idle),
                    );
                    append_batch_acked(
                        &session,
                        &wake_turn_id,
                        astrcode_core::AgentEventContext::default(),
                        MailboxBatchAckedPayload {
                            target_agent_id: wake_target_agent_id.clone(),
                            turn_id: wake_turn_id.clone(),
                            batch_id: wake_batch_id.clone(),
                            delivery_ids: wake_delivery_ids.clone(),
                        },
                        &mut translator,
                    )
                    .await
                }
                .await;
                match ack_result {
                    Ok(_) => {
                        let consumed = service
                            .runtime
                            .agent_control
                            .consume_parent_delivery_batch(&wake_session_id, &wake_delivery_ids)
                            .await;
                        if !consumed {
                            log::warn!(
                                "parent wake turn succeeded but delivery batch consume failed: \
                                 parentSession='{}', turnId='{}', batchId='{}'",
                                wake_session_id,
                                wake_turn_id,
                                wake_batch_id
                            );
                        }
                        service
                            .runtime
                            .observability
                            .record_delivery_buffer(DeliveryBufferStage::Dequeued);
                        service
                            .runtime
                            .observability
                            .record_child_lifecycle(ChildLifecycleStage::ReactivationSucceeded);
                        service
                            .runtime
                            .observability
                            .record_delivery_buffer(DeliveryBufferStage::WakeSucceeded);
                        should_continue_draining = true;
                    },
                    Err(error) => {
                        let _ = service
                            .runtime
                            .agent_control
                            .requeue_parent_delivery_batch(&wake_session_id, &wake_delivery_ids)
                            .await;
                        log::warn!(
                            "parent wake turn succeeded but mailbox ack append failed: \
                             parentSession='{}', turnId='{}', batchId='{}', error='{}'",
                            wake_session_id,
                            wake_turn_id,
                            wake_batch_id,
                            error
                        );
                        service
                            .runtime
                            .observability
                            .record_child_lifecycle(ChildLifecycleStage::ReactivationFailed);
                        service
                            .runtime
                            .observability
                            .record_delivery_buffer(DeliveryBufferStage::WakeFailed);
                    },
                }
            } else {
                log::warn!(
                    "parent wake turn finished with failure, requeueing delivery batch: \
                     parentSession='{}', turnId='{}', batchId='{}', deliveryCount={}",
                    wake_session_id,
                    wake_turn_id,
                    wake_batch_id,
                    wake_delivery_ids.len()
                );
                // 重新排队 delivery，以便后续重试
                let requeued = service
                    .runtime
                    .agent_control
                    .requeue_parent_delivery_batch(&wake_session_id, &wake_delivery_ids)
                    .await;
                if requeued != wake_delivery_ids.len() {
                    log::error!(
                        "parent wake turn failed and delivery batch requeue was incomplete: \
                         parentSession='{}', turnId='{}', batchId='{}', expected={}, actual={}",
                        wake_session_id,
                        wake_turn_id,
                        wake_batch_id,
                        wake_delivery_ids.len(),
                        requeued
                    );
                }
                service
                    .runtime
                    .observability
                    .record_child_lifecycle(ChildLifecycleStage::ReactivationFailed);
                service
                    .runtime
                    .observability
                    .record_delivery_buffer(DeliveryBufferStage::WakeFailed);
            }
            observability.record_turn_execution(turn_started_at.elapsed(), result.succeeded);
            if should_continue_draining {
                let runtime_handle = tokio::runtime::Handle::current();
                let drain_service = service.clone();
                let drain_session_id = wake_session_id.clone();
                if let Err(error) =
                    spawn_blocking_service("drain parent delivery queue", move || {
                        runtime_handle.block_on(
                            drain_service.try_start_parent_delivery_turn(&drain_session_id),
                        )
                    })
                    .await
                {
                    log::warn!(
                        "failed to continue draining parent delivery queue: parentSession='{}', \
                         error='{}'",
                        wake_session_id,
                        error
                    );
                }
            }
        });
        // Why: wake turn 是运行时桥接任务，必须把 JoinHandle 收进统一注册表，
        // 避免 detached task 在关闭/测试场景里悄悄丢失。
        self.runtime.lifecycle().register_turn_task(handle);

        Ok(true)
    }

    async fn append_parent_delivery_mailbox_queue(
        &self,
        parent_session: &astrcode_runtime_session::SessionState,
        parent_turn_id: &str,
        notification: &astrcode_core::ChildSessionNotification,
    ) -> ServiceResult<()> {
        let Some(target_agent_id) = notification.child_ref.parent_agent_id.clone() else {
            return Err(ServiceError::InvalidInput(
                "child terminal delivery missing direct parent agent id".to_string(),
            ));
        };
        let message = notification
            .final_reply_excerpt
            .as_deref()
            .filter(|excerpt| !excerpt.trim().is_empty())
            .unwrap_or(notification.summary.as_str())
            .to_string();
        let sender_last_turn_outcome = match notification.status {
            astrcode_core::AgentStatus::Completed => Some(AgentTurnOutcome::Completed),
            astrcode_core::AgentStatus::Failed => Some(AgentTurnOutcome::Failed),
            astrcode_core::AgentStatus::Cancelled => Some(AgentTurnOutcome::Cancelled),
            astrcode_core::AgentStatus::TokenExceeded => Some(AgentTurnOutcome::TokenExceeded),
            astrcode_core::AgentStatus::Pending | astrcode_core::AgentStatus::Running => None,
        };
        let payload = MailboxQueuedPayload {
            envelope: AgentMailboxEnvelope {
                delivery_id: notification.notification_id.clone(),
                from_agent_id: notification.child_ref.agent_id.clone(),
                to_agent_id: target_agent_id,
                message,
                queued_at: chrono::Utc::now(),
                sender_lifecycle_status: AgentLifecycleStatus::Idle,
                sender_last_turn_outcome,
                sender_open_session_id: notification.child_ref.open_session_id.clone(),
            },
        };
        let mut translator = EventTranslator::new(parent_session.current_phase()?);
        append_mailbox_queued(
            parent_session,
            parent_turn_id,
            astrcode_core::AgentEventContext::default(),
            payload,
            &mut translator,
        )
        .await
        .map_err(|error| ServiceError::Internal(AstrError::Internal(error.to_string())))?;
        Ok(())
    }
}

fn build_parent_delivery_prompt_declaration(delivery: &PendingParentDelivery) -> PromptDeclaration {
    PromptDeclaration {
        block_id: format!("runtime.parent_delivery.{}", delivery.delivery_id),
        title: "Child Session Delivery".to_string(),
        content: astrcode_runtime_agent_loop::child_delivery_prompt_declaration(
            &delivery.notification,
        ),
        render_target: PromptDeclarationRenderTarget::System,
        layer: PromptLayer::Dynamic,
        kind: PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(900),
        always_include: true,
        source: PromptDeclarationSource::Builtin,
        capability_name: Some("spawnAgent".to_string()),
        origin: Some(format!(
            "parent-delivery:{}:{}",
            delivery.parent_turn_id, delivery.delivery_id
        )),
    }
}

fn build_parent_delivery_prompt_declarations(
    deliveries: &[PendingParentDelivery],
) -> Vec<PromptDeclaration> {
    deliveries
        .iter()
        .map(build_parent_delivery_prompt_declaration)
        .collect()
}

#[async_trait]
impl ExecutionOrchestrationBoundary for AgentExecutionServiceHandle {
    async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> std::result::Result<astrcode_core::ExecutionAccepted, AstrError> {
        AgentExecutionServiceHandle::submit_prompt(self, session_id, text)
            .await
            .map_err(service_error_to_astr)
    }

    async fn interrupt_session(&self, session_id: &str) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::interrupt_session(self, session_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn execute_root_agent(
        &self,
        agent_id: String,
        task: String,
        context: Option<String>,
        context_overrides: Option<astrcode_core::SubagentContextOverrides>,
        working_dir: std::path::PathBuf,
    ) -> std::result::Result<astrcode_core::ExecutionAccepted, AstrError> {
        AgentExecutionServiceHandle::execute_root_agent(
            self,
            agent_id,
            task,
            context,
            context_overrides,
            working_dir,
        )
        .await
        .map_err(service_error_to_astr)
    }
}

#[async_trait]
impl LiveSubRunControlBoundary for AgentExecutionServiceHandle {
    async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError> {
        AgentExecutionServiceHandle::get_subrun_handle(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn cancel_subrun(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::cancel_subrun(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn launch_subagent(
        &self,
        params: SpawnAgentParams,
        ctx: &ToolContext,
    ) -> std::result::Result<SubRunResult, AstrError> {
        AgentExecutionServiceHandle::launch_subagent(self, params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn list_profiles(&self) -> std::result::Result<Vec<AgentProfile>, AstrError> {
        Ok(self
            .runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests;
