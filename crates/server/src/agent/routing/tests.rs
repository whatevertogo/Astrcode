use std::time::{Duration, Instant};

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, CancelToken, CloseAgentParams,
    CompletedParentDeliveryPayload, ObserveParams, ParentDeliveryPayload, SendAgentParams,
    SendToChildParams, SendToParentParams, SessionId, SpawnAgentParams, StorageEventPayload,
    ToolContext,
};
use astrcode_host_session::{CollaborationExecutor, SubAgentExecutor};
use tokio::time::sleep;

use super::super::{root_execution_event_context, subrun_event_context};
use crate::{
    agent::test_support::{TestLlmBehavior, build_agent_test_harness},
    lifecycle::governance::ObservabilitySnapshotProvider,
};

async fn spawn_direct_child(
    harness: &crate::agent::test_support::AgentTestHarness,
    parent_session_id: &str,
    working_dir: &std::path::Path,
) -> (String, String) {
    harness
        .session_runtime
        .agent_control()
        .register_root_agent(
            "root-agent".to_string(),
            parent_session_id.to_string(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");
    let parent_ctx = ToolContext::new(
        parent_session_id.to_string().into(),
        working_dir.to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    let launched = harness
        .service
        .launch(
            SpawnAgentParams {
                r#type: Some("reviewer".to_string()),
                description: "检查 crates".to_string(),
                prompt: "请检查 crates 目录".to_string(),
                context: None,
            },
            &parent_ctx,
        )
        .await
        .expect("spawn should succeed");
    let child_agent_id = launched
        .handoff()
        .and_then(|handoff| {
            handoff
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "agent")
                .map(|artifact| artifact.id.clone())
        })
        .expect("child agent artifact should exist");
    for _ in 0..20 {
        if harness
            .session_runtime
            .get_lifecycle(&child_agent_id)
            .await
            .is_some_and(|lifecycle| lifecycle == astrcode_core::AgentLifecycleStatus::Idle)
        {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    (child_agent_id, parent_ctx.session_id().to_string())
}

#[tokio::test]
async fn collaboration_calls_reject_non_direct_child() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");

    let parent_a = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session A should be created");
    let (child_agent_id, _) =
        spawn_direct_child(&harness, &parent_a.session_id, project.path()).await;

    let parent_b = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session B should be created");
    harness
        .session_runtime
        .agent_control()
        .register_root_agent(
            "other-root".to_string(),
            parent_b.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("other root agent should be registered");
    let other_ctx = ToolContext::new(
        parent_b.session_id.clone().into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-other")
    .with_agent_context(root_execution_event_context("other-root", "root-profile"));

    let send_error = harness
        .service
        .send(
            SendAgentParams::ToChild(SendToChildParams {
                agent_id: child_agent_id.clone().into(),
                message: "继续".to_string(),
                context: None,
            }),
            &other_ctx,
        )
        .await
        .expect_err("send should reject non-direct child");
    assert!(send_error.to_string().contains("direct child"));

    let observe_error = harness
        .service
        .observe(
            ObserveParams {
                agent_id: child_agent_id.clone(),
            },
            &other_ctx,
        )
        .await
        .expect_err("observe should reject non-direct child");
    assert!(observe_error.to_string().contains("direct child"));

    let close_error = harness
        .service
        .close(
            CloseAgentParams {
                agent_id: child_agent_id.into(),
            },
            &other_ctx,
        )
        .await
        .expect_err("close should reject non-direct child");
    assert!(close_error.to_string().contains("direct child"));

    let parent_b_events = harness
        .session_runtime
        .replay_stored_events(&SessionId::from(parent_b.session_id.clone()))
        .await
        .expect("other parent events should replay");
    assert!(parent_b_events.iter().any(|stored| matches!(
        &stored.event.payload,
        StorageEventPayload::AgentCollaborationFact { fact, .. }
            if fact.action == AgentCollaborationActionKind::Send
                && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                && fact.reason_code.as_deref() == Some("ownership_mismatch")
    )));
}

#[tokio::test]
async fn send_to_idle_child_reports_resume_semantics() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    let (child_agent_id, parent_session_id) =
        spawn_direct_child(&harness, &parent.session_id, project.path()).await;
    let parent_ctx = ToolContext::new(
        parent_session_id.into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent-2")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    let result = harness
        .service
        .send(
            SendAgentParams::ToChild(SendToChildParams {
                agent_id: child_agent_id.into(),
                message: "请继续整理结论".to_string(),
                context: None,
            }),
            &parent_ctx,
        )
        .await
        .expect("send should succeed");

    assert_eq!(result.delivery_id(), None);
    assert!(
        result
            .summary()
            .is_some_and(|summary| summary.contains("已恢复"))
    );
    assert_eq!(
        result
            .delegation()
            .map(|metadata| metadata.responsibility_summary.as_str()),
        Some("检查 crates"),
        "resumed child should keep the original responsibility branch metadata"
    );
    assert_eq!(
        result
            .child_agent_ref()
            .map(|child_ref| child_ref.lineage_kind),
        Some(astrcode_core::ChildSessionLineageKind::Resume),
        "resumed child projection should expose resume lineage instead of masquerading as spawn"
    );
    let resumed_child = harness
        .session_runtime
        .get_agent_handle(
            result
                .child_agent_ref()
                .map(|child_ref| child_ref.agent_id().as_str())
                .expect("child ref should exist"),
        )
        .await
        .expect("resumed child handle should exist");
    assert_eq!(resumed_child.parent_turn_id, "turn-parent-2".into());
    assert_eq!(
        resumed_child.lineage_kind,
        astrcode_core::ChildSessionLineageKind::Resume
    );
}

#[tokio::test]
async fn send_to_running_child_reports_input_queue_semantics() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    let (child_agent_id, parent_session_id) =
        spawn_direct_child(&harness, &parent.session_id, project.path()).await;
    let parent_ctx = ToolContext::new(
        parent_session_id.into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent-3")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    let _ = harness
        .session_runtime
        .agent_control()
        .set_lifecycle(
            &child_agent_id,
            astrcode_core::AgentLifecycleStatus::Running,
        )
        .await;

    let result = harness
        .service
        .send(
            SendAgentParams::ToChild(SendToChildParams {
                agent_id: child_agent_id.into(),
                message: "继续第二轮".to_string(),
                context: Some("只看 CI".to_string()),
            }),
            &parent_ctx,
        )
        .await
        .expect("send should succeed");

    assert!(result.delivery_id().is_some());
    assert!(
        result
            .summary()
            .is_some_and(|summary| summary.contains("input queue 排队"))
    );
}

#[tokio::test]
async fn send_to_parent_rejects_root_execution_without_direct_parent() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    harness
        .session_runtime
        .agent_control()
        .register_root_agent(
            "root-agent".to_string(),
            parent.session_id.clone(),
            "root-profile".to_string(),
        )
        .await
        .expect("root agent should be registered");

    let root_ctx = ToolContext::new(
        parent.session_id.clone().into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-root")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    let error = harness
        .service
        .send(
            SendAgentParams::ToParent(SendToParentParams {
                payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                    message: "根节点不应该上行".to_string(),
                    findings: Vec::new(),
                    artifacts: Vec::new(),
                }),
            }),
            &root_ctx,
        )
        .await
        .expect_err("root agent should not be able to send upward");
    assert!(error.to_string().contains("no direct parent"));

    let events = harness
        .session_runtime
        .replay_stored_events(&SessionId::from(parent.session_id.clone()))
        .await
        .expect("parent events should replay");
    assert!(events.iter().any(|stored| matches!(
        &stored.event.payload,
        StorageEventPayload::AgentCollaborationFact { fact, .. }
            if fact.action == AgentCollaborationActionKind::Delivery
                && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                && fact.reason_code.as_deref() == Some("missing_direct_parent")
    )));
}

#[tokio::test]
async fn send_to_parent_from_resumed_child_routes_to_current_parent_turn() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    let (child_agent_id, parent_session_id) =
        spawn_direct_child(&harness, &parent.session_id, project.path()).await;
    let parent_ctx = ToolContext::new(
        parent_session_id.into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent-2")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    harness
        .service
        .send(
            SendAgentParams::ToChild(SendToChildParams {
                agent_id: child_agent_id.clone().into(),
                message: "继续整理并向我汇报".to_string(),
                context: None,
            }),
            &parent_ctx,
        )
        .await
        .expect("send should resume idle child");

    let resumed_child = harness
        .session_runtime
        .get_agent_handle(&child_agent_id)
        .await
        .expect("resumed child handle should exist");
    let child_ctx = ToolContext::new(
        resumed_child
            .child_session_id
            .clone()
            .expect("child session id should exist"),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-child-report-2")
    .with_agent_context(subrun_event_context(&resumed_child));
    let metrics_before = harness.metrics.snapshot();

    let result = harness
        .service
        .send(
            SendAgentParams::ToParent(SendToParentParams {
                payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                    message: "继续推进后的显式上报".to_string(),
                    findings: Vec::new(),
                    artifacts: Vec::new(),
                }),
            }),
            &child_ctx,
        )
        .await
        .expect("resumed child should be able to send upward");

    assert!(result.delivery_id().is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay during wake wait");
        if parent_events.iter().any(|stored| {
            matches!(
                &stored.event.payload,
                StorageEventPayload::UserMessage { content, origin, .. }
                    if *origin == astrcode_core::UserMessageOrigin::QueuedInput
                        && content.contains("继续推进后的显式上报")
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "explicit upstream send should trigger parent wake and consume the queued input"
        );
        sleep(Duration::from_millis(20)).await;
    }

    let parent_events = harness
        .session_runtime
        .replay_stored_events(&SessionId::from(parent.session_id.clone()))
        .await
        .expect("parent events should replay");
    assert!(parent_events.iter().any(|stored| matches!(
        &stored.event.payload,
        StorageEventPayload::ChildSessionNotification { notification, .. }
            if stored.event.turn_id.as_deref() == Some("turn-parent-2")
                && notification.child_ref.sub_run_id() == &resumed_child.sub_run_id
                && notification.child_ref.lineage_kind
                    == astrcode_core::ChildSessionLineageKind::Resume
                && notification.delivery.as_ref().is_some_and(|delivery| {
                    delivery.origin == astrcode_core::ParentDeliveryOrigin::Explicit
                        && delivery.payload.message() == "继续推进后的显式上报"
                })
    )));
    assert!(
        !parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::ChildSessionNotification { notification, .. }
                if stored.event.turn_id.as_deref() == Some("turn-parent")
                    && notification.delivery.as_ref().is_some_and(|delivery| {
                        delivery.payload.message() == "继续推进后的显式上报"
                    })
        )),
        "resumed child delivery must target the current parent turn instead of the stale spawn \
         turn"
    );
    assert!(
        parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentInputQueued { payload }
                if payload.envelope.message == "继续推进后的显式上报"
        )),
        "explicit upstream send should enqueue the same delivery for parent wake consumption"
    );
    assert!(
        parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::UserMessage { content, origin, .. }
                if *origin == astrcode_core::UserMessageOrigin::QueuedInput
                    && content.contains("继续推进后的显式上报")
        )),
        "parent wake turn should consume the explicit upstream delivery as queued input"
    );
    let metrics = harness.metrics.snapshot();
    assert!(
        metrics.execution_diagnostics.parent_reactivation_requested
            >= metrics_before
                .execution_diagnostics
                .parent_reactivation_requested
    );
}

#[tokio::test]
async fn send_to_parent_rejects_when_direct_parent_is_terminated() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    let (child_agent_id, _) =
        spawn_direct_child(&harness, &parent.session_id, project.path()).await;
    let child_handle = harness
        .session_runtime
        .get_agent_handle(&child_agent_id)
        .await
        .expect("child handle should exist");

    let _ = harness
        .session_runtime
        .agent_control()
        .set_lifecycle(
            "root-agent",
            astrcode_core::AgentLifecycleStatus::Terminated,
        )
        .await;

    let child_ctx = ToolContext::new(
        child_handle
            .child_session_id
            .clone()
            .expect("child session id should exist"),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-child-report")
    .with_agent_context(subrun_event_context(&child_handle));

    let error = harness
        .service
        .send(
            SendAgentParams::ToParent(SendToParentParams {
                payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                    message: "父级已终止".to_string(),
                    findings: Vec::new(),
                    artifacts: Vec::new(),
                }),
            }),
            &child_ctx,
        )
        .await
        .expect_err("terminated parent should reject upward send");
    assert!(error.to_string().contains("terminated"));

    let parent_events = harness
        .session_runtime
        .replay_stored_events(&SessionId::from(parent.session_id.clone()))
        .await
        .expect("parent events should replay");
    assert!(parent_events.iter().any(|stored| matches!(
        &stored.event.payload,
        StorageEventPayload::AgentCollaborationFact { fact, .. }
            if fact.action == AgentCollaborationActionKind::Delivery
                && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                && fact.reason_code.as_deref() == Some("parent_terminated")
    )));
}

#[tokio::test]
async fn close_reports_cascade_scope_for_descendants() {
    let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
        content: "完成。".to_string(),
    })
    .expect("test harness should build");
    let project = tempfile::tempdir().expect("tempdir should be created");
    let parent = harness
        .session_runtime
        .create_session(project.path().display().to_string())
        .await
        .expect("parent session should be created");
    let (child_agent_id, parent_session_id) =
        spawn_direct_child(&harness, &parent.session_id, project.path()).await;

    let child_handle = harness
        .session_runtime
        .agent()
        .get_handle(&child_agent_id)
        .await
        .expect("child handle should exist");
    let child_ctx = ToolContext::new(
        child_handle
            .child_session_id
            .clone()
            .expect("child session id should exist"),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-child-1")
    .with_agent_context(subrun_event_context(&child_handle));
    let _grandchild = harness
        .service
        .launch(
            SpawnAgentParams {
                r#type: Some("reviewer".to_string()),
                description: "进一步检查".to_string(),
                prompt: "请进一步检查测试覆盖".to_string(),
                context: None,
            },
            &child_ctx,
        )
        .await
        .expect("grandchild spawn should succeed");

    let parent_ctx = ToolContext::new(
        parent_session_id.into(),
        project.path().to_path_buf(),
        CancelToken::new(),
    )
    .with_turn_id("turn-parent-close")
    .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

    let result = harness
        .service
        .close(
            CloseAgentParams {
                agent_id: child_agent_id.into(),
            },
            &parent_ctx,
        )
        .await
        .expect("close should succeed");

    assert_eq!(result.cascade(), Some(true));
    assert!(
        result
            .summary()
            .is_some_and(|summary| summary.contains("1 个后代"))
    );
}
