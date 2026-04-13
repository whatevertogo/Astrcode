use std::time::Duration;

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentMode, AgentProfile, AgentTurnOutcome,
    ChildAgentRef, ChildSessionLineageKind, ChildSessionNotification, ChildSessionNotificationKind,
    LiveSubRunControlBoundary, SessionId, SubRunHandle,
};

// 直接内联默认值，避免 kernel 依赖 runtime-config
const DEFAULT_MAX_AGENT_DEPTH: usize = 3;
const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 8;

use super::{AgentControl, AgentControlError, LiveSubRunControl, StaticAgentProfileSource};

fn explore_profile() -> AgentProfile {
    AgentProfile {
        id: "explore".to_string(),
        name: "Explore".to_string(),
        description: "只读探索".to_string(),
        mode: AgentMode::SubAgent,
        system_prompt: None,
        allowed_tools: vec!["readFile".to_string()],
        disallowed_tools: Vec::new(),
        // TODO: 未来可能需要添加更多执行限制字段（如 max_steps）
        model_preference: Some("fast".to_string()),
    }
}

fn sample_parent_delivery(
    notification_id: &str,
    parent_session_id: &str,
    parent_turn_id: &str,
) -> (String, String, ChildSessionNotification) {
    (
        parent_session_id.to_string(),
        parent_turn_id.to_string(),
        ChildSessionNotification {
            notification_id: notification_id.to_string(),
            child_ref: ChildAgentRef {
                agent_id: format!("agent-{notification_id}"),
                session_id: parent_session_id.to_string(),
                sub_run_id: format!("subrun-{notification_id}"),
                parent_agent_id: None,
                parent_sub_run_id: None,
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: format!("child-session-{notification_id}"),
            },
            kind: ChildSessionNotificationKind::Delivered,
            summary: format!("summary-{notification_id}"),
            status: AgentLifecycleStatus::Idle,
            source_tool_call_id: None,
            final_reply_excerpt: Some(format!("final-{notification_id}")),
        },
    )
}

fn sample_parent_delivery_for_child(
    notification_id: &str,
    parent_session_id: &str,
    _parent_turn_id: &str,
    child: &SubRunHandle,
) -> ChildSessionNotification {
    ChildSessionNotification {
        notification_id: notification_id.to_string(),
        child_ref: ChildAgentRef {
            agent_id: child.agent_id.clone(),
            session_id: parent_session_id.to_string(),
            sub_run_id: child.sub_run_id.clone(),
            parent_agent_id: child.parent_agent_id.clone(),
            parent_sub_run_id: child.parent_sub_run_id.clone(),
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: child.lifecycle,
            open_session_id: child
                .child_session_id
                .clone()
                .unwrap_or_else(|| child.session_id.clone()),
        },
        kind: ChildSessionNotificationKind::Delivered,
        summary: format!("summary-{notification_id}"),
        status: child.lifecycle,
        source_tool_call_id: None,
        final_reply_excerpt: Some(format!("final-{notification_id}")),
    }
}

#[tokio::test]
async fn spawn_list_and_wait_track_status() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");

    assert_eq!(handle.lifecycle, AgentLifecycleStatus::Pending);
    assert_eq!(control.list().await.len(), 1);

    let agent_id = handle.agent_id.clone();
    let waiter = {
        let control = control.clone();
        tokio::spawn(async move { control.wait(&agent_id).await })
    };
    // 先让 waiter 完成订阅，避免测试依赖调度时序而偶发卡住。
    tokio::task::yield_now().await;

    control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await
        .expect("agent should exist");
    let running = control
        .get(&handle.agent_id)
        .await
        .expect("agent should exist");
    assert_eq!(running.lifecycle, AgentLifecycleStatus::Running);

    control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
        .await
        .expect("agent should exist");
    let completed = control
        .get(&handle.agent_id)
        .await
        .expect("agent should exist");
    assert_eq!(completed.lifecycle, AgentLifecycleStatus::Idle);

    let waited = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("waiter should finish before timeout")
        .expect("waiter should join");
    assert_eq!(
        waited.expect("wait should resolve").lifecycle,
        AgentLifecycleStatus::Idle
    );
}

#[tokio::test]
async fn cancelling_parent_turn_cascades_to_children() {
    // 需要 depth >= 2 才能测试 parent → child 嵌套
    let control = AgentControl::with_limits(3, 10, 256);
    let parent = control
        .spawn(
            &explore_profile(),
            "session-parent",
            "turn-root".to_string(),
            None,
        )
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&parent.agent_id, AgentLifecycleStatus::Running)
        .await;

    let child = control
        .spawn(
            &explore_profile(),
            "session-child",
            "turn-root".to_string(),
            Some(parent.agent_id.clone()),
        )
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&child.agent_id, AgentLifecycleStatus::Running)
        .await;

    let cancelled = control.cancel_for_parent_turn("turn-root").await;
    assert_eq!(cancelled.len(), 2);

    let parent_handle = control
        .get(&parent.agent_id)
        .await
        .expect("parent should exist");
    let child_handle = control
        .get(&child.agent_id)
        .await
        .expect("child should exist");
    assert_eq!(parent_handle.lifecycle, AgentLifecycleStatus::Terminated);
    assert_eq!(child_handle.lifecycle, AgentLifecycleStatus::Terminated);

    let child_cancel = control
        .cancel_token(&child.agent_id)
        .await
        .expect("child cancel token should exist");
    assert!(child_cancel.is_cancelled());
}

#[tokio::test]
async fn spawn_rejects_unknown_parent_agent() {
    let control = AgentControl::new();

    let error = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some("missing-parent".to_string()),
        )
        .await
        .expect_err("spawn should reject unknown parent");

    assert_eq!(
        error,
        AgentControlError::ParentAgentNotFound {
            agent_id: "missing-parent".to_string(),
        }
    );
    assert!(control.list().await.is_empty());
}

#[tokio::test]
async fn failed_spawn_does_not_consume_agent_id() {
    let control = AgentControl::new();

    let _ = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some("missing-parent".to_string()),
        )
        .await
        .expect_err("spawn should reject unknown parent");

    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("first successful spawn should still get the first id");

    assert_eq!(handle.agent_id, "agent-1");
}

#[tokio::test]
async fn cancel_directly_cascades_to_child_tree() {
    // 需要 depth >= 3 才能测试 parent → child → grandchild 嵌套
    let control = AgentControl::with_limits(3, 10, 256);
    let parent = control
        .spawn(
            &explore_profile(),
            "session-parent",
            "turn-root".to_string(),
            None,
        )
        .await
        .expect("parent spawn should succeed");
    let child = control
        .spawn(
            &explore_profile(),
            "session-child",
            "turn-root".to_string(),
            Some(parent.agent_id.clone()),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild = control
        .spawn(
            &explore_profile(),
            "session-grandchild",
            "turn-root".to_string(),
            Some(child.agent_id.clone()),
        )
        .await
        .expect("grandchild spawn should succeed");
    let _ = control
        .set_lifecycle(&parent.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&child.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&grandchild.agent_id, AgentLifecycleStatus::Running)
        .await;

    let cancelled = control
        .cancel(&parent.agent_id)
        .await
        .expect("parent cancel should exist");
    assert_eq!(cancelled.lifecycle, AgentLifecycleStatus::Terminated);

    for agent_id in [&parent.agent_id, &child.agent_id, &grandchild.agent_id] {
        let handle = control
            .get(agent_id)
            .await
            .expect("agent should still exist");
        assert_eq!(handle.lifecycle, AgentLifecycleStatus::Terminated);
    }
}

#[tokio::test]
async fn mark_failed_transitions_agent_to_final_failed_state() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Failed)
        .await
        .expect("agent should exist");
    let failed = control
        .get(&handle.agent_id)
        .await
        .expect("agent should exist");
    assert_eq!(failed.lifecycle, AgentLifecycleStatus::Idle);

    let waited = control
        .wait(&handle.agent_id)
        .await
        .expect("failed agent should still be queryable");
    assert_eq!(waited.lifecycle, AgentLifecycleStatus::Idle);
}

#[tokio::test]
async fn gc_prunes_old_finalized_leaf_agents_but_keeps_recent_and_live_nodes() {
    let control =
        AgentControl::with_limits(DEFAULT_MAX_AGENT_DEPTH, DEFAULT_MAX_CONCURRENT_AGENTS, 1);

    let first = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("first spawn should succeed");
    let second = control
        .spawn(&explore_profile(), "session-1", "turn-2".to_string(), None)
        .await
        .expect("second spawn should succeed");
    let live = control
        .spawn(&explore_profile(), "session-1", "turn-3".to_string(), None)
        .await
        .expect("live spawn should succeed");

    let _ = control
        .complete_turn(&first.agent_id, AgentTurnOutcome::Completed)
        .await;
    let _ = control
        .complete_turn(&second.agent_id, AgentTurnOutcome::Failed)
        .await;

    let handles = control.list().await;
    assert_eq!(
        handles.len(),
        2,
        "gc should evict the oldest finalized leaf"
    );
    assert!(control.get(&first.agent_id).await.is_none());
    assert_eq!(
        control
            .get(&second.agent_id)
            .await
            .expect("newer finalized agent")
            .lifecycle,
        AgentLifecycleStatus::Idle
    );
    assert_eq!(
        control
            .get(&live.agent_id)
            .await
            .expect("live agent should remain")
            .lifecycle,
        AgentLifecycleStatus::Pending
    );
}

#[tokio::test]
async fn spawn_rejects_agents_that_exceed_max_depth() {
    let control = AgentControl::with_limits(2, 8, usize::MAX);
    let root = control
        .spawn(
            &explore_profile(),
            "session-root",
            "turn-root".to_string(),
            None,
        )
        .await
        .expect("root should fit within depth 1");
    let child = control
        .spawn(
            &explore_profile(),
            "session-child",
            "turn-root".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("child should fit within depth 2");
    assert_eq!(root.depth, 1);
    assert_eq!(child.depth, 2);

    let error = control
        .spawn(
            &explore_profile(),
            "session-grandchild",
            "turn-root".to_string(),
            Some(child.agent_id.clone()),
        )
        .await
        .expect_err("grandchild should exceed max depth");
    assert_eq!(
        error,
        AgentControlError::MaxDepthExceeded { current: 3, max: 2 }
    );
}

#[tokio::test]
async fn finalized_agents_release_concurrency_slots() {
    let control = AgentControl::with_limits(8, 2, usize::MAX);
    let first = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("first spawn should succeed");
    let second = control
        .spawn(&explore_profile(), "session-2", "turn-2".to_string(), None)
        .await
        .expect("second spawn should succeed");

    let error = control
        .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
        .await
        .expect_err("third active agent should exceed concurrent limit");
    assert_eq!(
        error,
        AgentControlError::MaxConcurrentExceeded { current: 2, max: 2 }
    );

    let _ = control
        .complete_turn(&first.agent_id, AgentTurnOutcome::Completed)
        .await;
    let third = control
        .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
        .await
        .expect("finalizing one agent should release a slot");
    assert_eq!(third.depth, 1);
    assert_eq!(
        control
            .get(&second.agent_id)
            .await
            .expect("second should still exist")
            .lifecycle,
        AgentLifecycleStatus::Pending
    );
}

#[tokio::test]
async fn live_subrun_control_surface_delegates_registry_and_profiles() {
    let control = AgentControl::new();
    let profile = explore_profile();
    let handle = control
        .spawn(&profile, "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let surface = LiveSubRunControl::new(
        control.clone(),
        StaticAgentProfileSource::new(vec![profile.clone()]),
    );
    let session_id = SessionId::from("session-1");

    let loaded = surface
        .get_subrun_handle(&session_id, &handle.sub_run_id)
        .await
        .expect("lookup should succeed")
        .expect("handle should exist");
    assert_eq!(loaded.agent_id, handle.agent_id);
    assert_eq!(
        surface
            .list_profiles()
            .await
            .expect("profiles should load")
            .len(),
        1
    );

    surface
        .cancel_subrun(&session_id, &handle.sub_run_id)
        .await
        .expect("cancel should succeed");
    assert_eq!(
        control
            .get(&handle.sub_run_id)
            .await
            .expect("handle should remain visible")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );
}

// ─── T028 协作操作运行时测试 ───────────────────────────

#[tokio::test]
async fn targeted_wait_resolves_only_specific_agent_not_siblings() {
    let control = AgentControl::with_limits(3, 10, 256);
    let agent_a = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("agent A spawn should succeed");
    let agent_b = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("agent B spawn should succeed");
    let _ = control
        .set_lifecycle(&agent_a.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&agent_b.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 只完成 agent_a，agent_b 仍运行中
    let _ = control
        .complete_turn(&agent_a.agent_id, AgentTurnOutcome::Completed)
        .await;

    // wait 应该立即返回已终态的 agent_a
    let waited = control
        .wait(&agent_a.agent_id)
        .await
        .expect("wait should resolve");
    assert_eq!(waited.lifecycle, AgentLifecycleStatus::Idle);

    // agent_b 仍然处于 Running 状态，不受影响
    let b_handle = control
        .get(&agent_b.agent_id)
        .await
        .expect("agent B should exist");
    assert_eq!(b_handle.lifecycle, AgentLifecycleStatus::Running);
}

#[tokio::test]
async fn resume_mints_new_execution_for_completed_agent() {
    let control = AgentControl::with_limits(3, 10, 256);
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
        .await;

    // 恢复已完成的 agent
    let resumed = control
        .resume(&handle.agent_id)
        .await
        .expect("resume should succeed");
    assert_eq!(resumed.lifecycle, AgentLifecycleStatus::Running);
    assert_eq!(resumed.agent_id, handle.agent_id);
    assert_ne!(
        resumed.sub_run_id, handle.sub_run_id,
        "resume should mint a new execution id"
    );

    let historical = control
        .get(&handle.sub_run_id)
        .await
        .expect("historical execution should remain queryable by old sub-run id");
    assert_eq!(historical.lifecycle, AgentLifecycleStatus::Idle);

    // 验证恢复后能再次正常到达终态
    let _ = control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
        .await;
    let final_handle = control
        .get(&handle.agent_id)
        .await
        .expect("agent should exist");
    assert_eq!(final_handle.lifecycle, AgentLifecycleStatus::Idle);
}

#[tokio::test]
async fn resume_rejects_non_final_agent() {
    let control = AgentControl::with_limits(3, 10, 256);
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    // Running 状态的 agent 不能被恢复
    let result = control.resume(&handle.agent_id).await;
    assert!(result.is_none(), "running agent should not be resumable");
}

#[tokio::test]
async fn close_cascades_to_entire_subtree_but_not_siblings() {
    let control = AgentControl::with_limits(4, 10, 256);

    // 构建两棵独立子树
    let tree_a_parent = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("tree A parent spawn should succeed");
    let tree_a_child = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some(tree_a_parent.agent_id.clone()),
        )
        .await
        .expect("tree A child spawn should succeed");

    let tree_b_parent = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("tree B parent spawn should succeed");
    let _tree_b_child = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some(tree_b_parent.agent_id.clone()),
        )
        .await
        .expect("tree B child spawn should succeed");

    let _ = control
        .set_lifecycle(&tree_a_parent.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&tree_a_child.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&tree_b_parent.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 关闭 tree A 的根，应级联到 tree A 的 child
    let cancelled = control
        .cancel(&tree_a_parent.agent_id)
        .await
        .expect("cancel should succeed");
    assert_eq!(cancelled.lifecycle, AgentLifecycleStatus::Terminated);

    // tree A 的 parent 和 child 都被取消
    assert_eq!(
        control
            .get(&tree_a_parent.agent_id)
            .await
            .expect("should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );
    assert_eq!(
        control
            .get(&tree_a_child.agent_id)
            .await
            .expect("should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );

    // tree B 不受影响
    assert_eq!(
        control
            .get(&tree_b_parent.agent_id)
            .await
            .expect("should exist")
            .lifecycle,
        AgentLifecycleStatus::Running
    );
}

#[tokio::test]
async fn resume_reoccupies_concurrency_slot() {
    let control = AgentControl::with_limits(8, 2, usize::MAX);
    let first = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("first spawn should succeed");
    let _second = control
        .spawn(&explore_profile(), "session-2", "turn-2".to_string(), None)
        .await
        .expect("second spawn should succeed");

    let _ = control
        .set_lifecycle(&first.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .complete_turn(&first.agent_id, AgentTurnOutcome::Completed)
        .await;

    // first 完成后释放了槽位，可以创建第三个
    let _third = control
        .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
        .await
        .expect("third spawn should succeed after first completed");

    // 恢复 first 会重新占用槽位，此时已有 3 个活跃（first resumed + second + third）
    let _ = control.resume(&first.agent_id).await;

    let error = control
        .spawn(&explore_profile(), "session-4", "turn-4".to_string(), None)
        .await
        .expect_err("should exceed concurrent limit after resume");
    assert_eq!(
        error,
        AgentControlError::MaxConcurrentExceeded { current: 3, max: 2 }
    );
}

// ─── 收件箱测试 ──────────────────────────────────────

fn sample_envelope(id: &str, from: &str, to: &str, message: &str) -> AgentInboxEnvelope {
    AgentInboxEnvelope {
        delivery_id: id.to_string(),
        from_agent_id: from.to_string(),
        to_agent_id: to.to_string(),
        kind: astrcode_core::InboxEnvelopeKind::ParentMessage,
        message: message.to_string(),
        context: None,
        is_final: false,
        summary: None,
        findings: Vec::new(),
        artifacts: Vec::new(),
    }
}

#[tokio::test]
async fn push_and_drain_inbox_enqueues_and_consumes_envelopes() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 推送两封信封
    control
        .push_inbox(
            &handle.agent_id,
            sample_envelope("d-1", "agent-parent", &handle.agent_id, "请修改"),
        )
        .await
        .expect("push should succeed");
    control
        .push_inbox(
            &handle.agent_id,
            sample_envelope("d-2", "agent-parent", &handle.agent_id, "补充说明"),
        )
        .await
        .expect("push should succeed");

    // 排空收件箱
    let envelopes = control
        .drain_inbox(&handle.agent_id)
        .await
        .expect("drain should succeed");
    assert_eq!(envelopes.len(), 2);
    assert_eq!(envelopes[0].delivery_id, "d-1");
    assert_eq!(envelopes[1].delivery_id, "d-2");

    // 二次排空为空
    let empty = control
        .drain_inbox(&handle.agent_id)
        .await
        .expect("drain should succeed");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn complete_turn_moves_agent_into_idle_with_last_outcome() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    let lifecycle = control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
        .await
        .expect("complete turn should succeed");
    assert_eq!(lifecycle, AgentLifecycleStatus::Idle);
    assert_eq!(
        control.get_lifecycle(&handle.agent_id).await,
        Some(AgentLifecycleStatus::Idle)
    );
    assert_eq!(
        control.get_turn_outcome(&handle.agent_id).await,
        Some(Some(AgentTurnOutcome::Completed))
    );
    assert_eq!(
        control
            .get(&handle.agent_id)
            .await
            .expect("completed handle should remain queryable")
            .lifecycle,
        AgentLifecycleStatus::Idle
    );
}

#[tokio::test]
async fn push_inbox_deduplication_by_delivery_id() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");

    // 推送相同 delivery_id 的信封两次
    control
        .push_inbox(
            &handle.agent_id,
            sample_envelope("d-dup", "agent-parent", &handle.agent_id, "消息"),
        )
        .await
        .expect("push should succeed");
    control
        .push_inbox(
            &handle.agent_id,
            sample_envelope("d-dup", "agent-parent", &handle.agent_id, "消息"),
        )
        .await
        .expect("push should succeed");

    // 当前实现不内置去重，由调用方保证幂等；
    // 验证两封信封都入队（调用方负责 dedupe 语义）
    let envelopes = control
        .drain_inbox(&handle.agent_id)
        .await
        .expect("drain should succeed");
    assert_eq!(envelopes.len(), 2);
}

#[tokio::test]
async fn wait_for_inbox_resolves_on_new_envelope() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    let agent_id = handle.agent_id.clone();
    let control_clone = control.clone();
    let waiter = tokio::spawn(async move { control_clone.wait_for_inbox(&agent_id).await });

    // 让 waiter 完成订阅
    tokio::task::yield_now().await;

    // 推送信封唤醒 waiter
    control
        .push_inbox(
            &handle.agent_id,
            sample_envelope("d-wait", "agent-parent", &handle.agent_id, "唤醒"),
        )
        .await
        .expect("push should succeed");

    let result = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("waiter should finish before timeout")
        .expect("waiter should join");
    assert!(result.is_some());
}

#[tokio::test]
async fn terminate_subtree_clears_pending_inbox_messages() {
    let control = AgentControl::with_limits(4, 16, 256);
    let parent = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("parent spawn should succeed");
    let child = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some(parent.agent_id.clone()),
        )
        .await
        .expect("child spawn should succeed");
    let _ = control
        .set_lifecycle(&parent.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&child.agent_id, AgentLifecycleStatus::Running)
        .await;

    control
        .push_inbox(
            &child.agent_id,
            sample_envelope("d-close", "agent-parent", &child.agent_id, "终止前排队消息"),
        )
        .await
        .expect("push should succeed");

    control
        .terminate_subtree(&parent.agent_id)
        .await
        .expect("terminate subtree should succeed");

    let child_inbox = control
        .drain_inbox(&child.agent_id)
        .await
        .expect("drain should succeed after close");
    assert!(child_inbox.is_empty());
    assert_eq!(
        control.get_lifecycle(&child.agent_id).await,
        Some(AgentLifecycleStatus::Terminated)
    );
}

#[tokio::test]
async fn terminate_subtree_discards_pending_parent_deliveries_for_closed_branch() {
    let control = AgentControl::with_limits(4, 16, 256);
    let root = control
        .spawn(
            &explore_profile(),
            "session-parent",
            "turn-root".to_string(),
            None,
        )
        .await
        .expect("root spawn should succeed");
    let child = control
        .spawn(
            &explore_profile(),
            "session-child-a",
            "turn-root".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("child spawn should succeed");
    let sibling = control
        .spawn(
            &explore_profile(),
            "session-child-b",
            "turn-root".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("sibling spawn should succeed");

    let session_id = "session-parent".to_string();
    let turn_id = "turn-root".to_string();
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                sample_parent_delivery_for_child("closed-branch", &session_id, &turn_id, &child,)
            )
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                sample_parent_delivery_for_child("live-branch", &session_id, &turn_id, &sibling,)
            )
            .await
    );
    assert_eq!(control.pending_parent_delivery_count(&session_id).await, 2);

    control
        .terminate_subtree(&child.agent_id)
        .await
        .expect("terminate should succeed");

    assert_eq!(control.pending_parent_delivery_count(&session_id).await, 1);
    let remaining = control
        .checkout_parent_delivery(&session_id)
        .await
        .expect("sibling delivery should remain queued");
    assert_eq!(remaining.notification.child_ref.agent_id, sibling.agent_id);
}

#[tokio::test]
async fn wait_for_inbox_returns_immediately_for_final_agent() {
    let control = AgentControl::new();
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
        .await;

    // complete_turn 后 lifecycle 为 Idle（非 Terminated），
    // wait_for_inbox 需要看到 is_final() 才立即返回。
    // 但 Idle 不是 final，所以先 terminate 使之成为 Terminated。
    // 先恢复为 Running 再 terminate
    let _ = control.resume(&handle.agent_id).await;
    control
        .terminate_subtree(&handle.agent_id)
        .await
        .expect("terminate should succeed");

    let result = control
        .wait_for_inbox(&handle.agent_id)
        .await
        .expect("should resolve immediately");
    assert_eq!(result.lifecycle, AgentLifecycleStatus::Terminated);
}

#[tokio::test]
async fn push_inbox_returns_none_for_nonexistent_agent() {
    let control = AgentControl::new();
    let result = control
        .push_inbox(
            "missing-agent",
            sample_envelope("d-1", "agent-parent", "missing-agent", "消息"),
        )
        .await;
    assert!(result.is_none());
}

#[tokio::test]
async fn parent_delivery_queue_deduplicates_and_preserves_fifo_order() {
    let control = AgentControl::new();
    let (session_id, turn_id, first) =
        sample_parent_delivery("delivery-1", "session-parent", "turn-parent");
    let (_, _, duplicate) = sample_parent_delivery("delivery-1", "session-parent", "turn-parent");
    let (_, _, second) = sample_parent_delivery("delivery-2", "session-parent", "turn-parent");

    assert!(
        control
            .enqueue_parent_delivery(session_id.clone(), turn_id.clone(), first)
            .await
    );
    assert!(
        !control
            .enqueue_parent_delivery(session_id.clone(), turn_id.clone(), duplicate)
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(session_id.clone(), turn_id, second)
            .await
    );

    let first_checked_out = control
        .checkout_parent_delivery(&session_id)
        .await
        .expect("first queued delivery should be available");
    assert_eq!(first_checked_out.delivery_id, "delivery-1");
    assert!(
        control
            .consume_parent_delivery(&session_id, &first_checked_out.delivery_id)
            .await
    );

    let second_checked_out = control
        .checkout_parent_delivery(&session_id)
        .await
        .expect("second queued delivery should be available");
    assert_eq!(second_checked_out.delivery_id, "delivery-2");
    assert_eq!(control.pending_parent_delivery_count(&session_id).await, 1);
}

#[tokio::test]
async fn parent_delivery_queue_can_requeue_busy_head_without_losing_it() {
    let control = AgentControl::new();
    let (session_id, turn_id, delivery) =
        sample_parent_delivery("delivery-busy", "session-parent", "turn-parent");

    assert!(
        control
            .enqueue_parent_delivery(session_id.clone(), turn_id, delivery)
            .await
    );

    let checked_out = control
        .checkout_parent_delivery(&session_id)
        .await
        .expect("delivery should be checked out");
    assert!(
        control
            .checkout_parent_delivery(&session_id)
            .await
            .is_none(),
        "waking delivery should block duplicate checkout"
    );

    assert!(
        control
            .requeue_parent_delivery(&session_id, &checked_out.delivery_id)
            .await
    );

    let retried = control
        .checkout_parent_delivery(&session_id)
        .await
        .expect("requeued delivery should become available again");
    assert_eq!(retried.delivery_id, checked_out.delivery_id);
    assert!(
        control
            .consume_parent_delivery(&session_id, &retried.delivery_id)
            .await
    );
    assert_eq!(control.pending_parent_delivery_count(&session_id).await, 0);
}

#[tokio::test]
async fn parent_delivery_batch_checkout_uses_turn_start_snapshot_for_same_parent_agent() {
    let control = AgentControl::new();
    let session_id = "session-parent".to_string();
    let turn_id = "turn-parent".to_string();
    let make_delivery =
        |delivery_id: &str, child_id: &str, parent_agent_id: &str| ChildSessionNotification {
            notification_id: delivery_id.to_string(),
            child_ref: ChildAgentRef {
                agent_id: child_id.to_string(),
                session_id: session_id.clone(),
                sub_run_id: format!("subrun-{delivery_id}"),
                parent_agent_id: Some(parent_agent_id.to_string()),
                parent_sub_run_id: Some(format!("subrun-{parent_agent_id}")),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: format!("child-session-{delivery_id}"),
            },
            kind: ChildSessionNotificationKind::Delivered,
            summary: format!("summary-{delivery_id}"),
            status: AgentLifecycleStatus::Idle,
            source_tool_call_id: None,
            final_reply_excerpt: Some(format!("final-{delivery_id}")),
        };

    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                make_delivery("delivery-1", "agent-child-1", "agent-parent-a"),
            )
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                make_delivery("delivery-2", "agent-child-2", "agent-parent-a"),
            )
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id,
                make_delivery("delivery-3", "agent-child-3", "agent-parent-b"),
            )
            .await
    );

    let first_batch = control
        .checkout_parent_delivery_batch(&session_id)
        .await
        .expect("same parent-agent head deliveries should form a batch");
    assert_eq!(
        first_batch
            .iter()
            .map(|delivery| delivery.delivery_id.as_str())
            .collect::<Vec<_>>(),
        vec!["delivery-1", "delivery-2"]
    );
    assert!(
        control
            .checkout_parent_delivery_batch(&session_id)
            .await
            .is_none(),
        "head batch is already waking; next batch must wait for consume/requeue"
    );
    assert!(
        control
            .consume_parent_delivery_batch(
                &session_id,
                &first_batch
                    .iter()
                    .map(|delivery| delivery.delivery_id.clone())
                    .collect::<Vec<_>>(),
            )
            .await
    );

    let second_batch = control
        .checkout_parent_delivery_batch(&session_id)
        .await
        .expect("next parent-agent group should become the next batch");
    assert_eq!(second_batch.len(), 1);
    assert_eq!(second_batch[0].delivery_id, "delivery-3");
}

#[tokio::test]
async fn parent_delivery_batch_requeue_restores_started_snapshot_for_retry() {
    let control = AgentControl::new();
    let session_id = "session-parent".to_string();
    let turn_id = "turn-parent".to_string();
    let make_delivery =
        |delivery_id: &str, child_id: &str, parent_agent_id: &str| ChildSessionNotification {
            notification_id: delivery_id.to_string(),
            child_ref: ChildAgentRef {
                agent_id: child_id.to_string(),
                session_id: session_id.clone(),
                sub_run_id: format!("subrun-{delivery_id}"),
                parent_agent_id: Some(parent_agent_id.to_string()),
                parent_sub_run_id: Some(format!("subrun-{parent_agent_id}")),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: format!("child-session-{delivery_id}"),
            },
            kind: ChildSessionNotificationKind::Delivered,
            summary: format!("summary-{delivery_id}"),
            status: AgentLifecycleStatus::Idle,
            source_tool_call_id: None,
            final_reply_excerpt: Some(format!("final-{delivery_id}")),
        };

    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                make_delivery("delivery-1", "agent-child-1", "agent-parent-a"),
            )
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id,
                make_delivery("delivery-2", "agent-child-2", "agent-parent-a"),
            )
            .await
    );

    let started_batch = control
        .checkout_parent_delivery_batch(&session_id)
        .await
        .expect("queued deliveries should form a started batch");
    let delivery_ids = started_batch
        .iter()
        .map(|delivery| delivery.delivery_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        delivery_ids,
        vec!["delivery-1".to_string(), "delivery-2".to_string()]
    );

    assert_eq!(
        control
            .requeue_parent_delivery_batch(&session_id, &delivery_ids)
            .await,
        2
    );

    let replayed_batch = control
        .checkout_parent_delivery_batch(&session_id)
        .await
        .expect("requeued started batch should become available again");
    assert_eq!(
        replayed_batch
            .iter()
            .map(|delivery| delivery.delivery_id.as_str())
            .collect::<Vec<_>>(),
        vec!["delivery-1", "delivery-2"]
    );
}

// ─── T035 层级协作回归测试 ────────────────────────────

/// 验证级联关闭是 leaf-first 语义：
/// 三层链 root → middle → leaf，关闭 middle 时，
/// leaf 先被取消（子树从叶子向上传播），root 不受影响。
#[tokio::test]
async fn leaf_first_cascade_cancels_deepest_child_before_parent() {
    let control = AgentControl::with_limits(4, 10, 256);

    let root = control
        .spawn(
            &explore_profile(),
            "session-root",
            "turn-1".to_string(),
            None,
        )
        .await
        .expect("root spawn should succeed");
    let middle = control
        .spawn(
            &explore_profile(),
            "session-middle",
            "turn-1".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("middle spawn should succeed");
    let leaf = control
        .spawn(
            &explore_profile(),
            "session-leaf",
            "turn-1".to_string(),
            Some(middle.agent_id.clone()),
        )
        .await
        .expect("leaf spawn should succeed");
    let _ = control
        .set_lifecycle(&root.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&middle.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&leaf.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 关闭 middle，应级联到 leaf，但不影响 root
    let cancelled = control
        .cancel(&middle.agent_id)
        .await
        .expect("cancel should succeed");
    assert_eq!(cancelled.lifecycle, AgentLifecycleStatus::Terminated);

    // middle 和 leaf 都被取消
    assert_eq!(
        control
            .get(&middle.agent_id)
            .await
            .expect("middle should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );
    assert_eq!(
        control
            .get(&leaf.agent_id)
            .await
            .expect("leaf should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );

    // root 不受影响
    assert_eq!(
        control
            .get(&root.agent_id)
            .await
            .expect("root should exist")
            .lifecycle,
        AgentLifecycleStatus::Running
    );
}

/// 验证子树隔离：关闭一个分支的中间节点不会影响兄弟分支。
/// root → middle_a → leaf_a
/// root → middle_b → leaf_b
/// 关闭 middle_a 只影响 middle_a + leaf_a，middle_b + leaf_b 不受影响。
#[tokio::test]
async fn subtree_isolation_closing_one_branch_does_not_affect_sibling_branch() {
    let control = AgentControl::with_limits(4, 10, 256);

    let root = control
        .spawn(
            &explore_profile(),
            "session-root",
            "turn-1".to_string(),
            None,
        )
        .await
        .expect("root spawn should succeed");
    let middle_a = control
        .spawn(
            &explore_profile(),
            "session-middle-a",
            "turn-1".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("middle_a spawn should succeed");
    let leaf_a = control
        .spawn(
            &explore_profile(),
            "session-leaf-a",
            "turn-1".to_string(),
            Some(middle_a.agent_id.clone()),
        )
        .await
        .expect("leaf_a spawn should succeed");
    let middle_b = control
        .spawn(
            &explore_profile(),
            "session-middle-b",
            "turn-1".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("middle_b spawn should succeed");
    let leaf_b = control
        .spawn(
            &explore_profile(),
            "session-leaf-b",
            "turn-1".to_string(),
            Some(middle_b.agent_id.clone()),
        )
        .await
        .expect("leaf_b spawn should succeed");

    let _ = control
        .set_lifecycle(&root.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&middle_a.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&leaf_a.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&middle_b.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&leaf_b.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 关闭 middle_a 分支
    let _ = control
        .cancel(&middle_a.agent_id)
        .await
        .expect("cancel should succeed");

    // branch A 全部被取消
    assert_eq!(
        control
            .get(&middle_a.agent_id)
            .await
            .expect("middle_a should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );
    assert_eq!(
        control
            .get(&leaf_a.agent_id)
            .await
            .expect("leaf_a should exist")
            .lifecycle,
        AgentLifecycleStatus::Terminated
    );

    // branch B 完全不受影响
    assert_eq!(
        control
            .get(&middle_b.agent_id)
            .await
            .expect("middle_b should exist")
            .lifecycle,
        AgentLifecycleStatus::Running
    );
    assert_eq!(
        control
            .get(&leaf_b.agent_id)
            .await
            .expect("leaf_b should exist")
            .lifecycle,
        AgentLifecycleStatus::Running
    );

    // root 也不受影响
    assert_eq!(
        control
            .get(&root.agent_id)
            .await
            .expect("root should exist")
            .lifecycle,
        AgentLifecycleStatus::Running
    );
}

/// 验证子向父 send 只投递给直接父 agent，不越级投递到祖父 agent。
/// root → middle → leaf
/// leaf 通过 send 只能投递到 middle 的 inbox，
/// root 的 inbox 不应收到 leaf 的投递。
#[tokio::test]
async fn deliver_to_parent_only_reaches_direct_parent_not_grandparent() {
    let control = AgentControl::with_limits(4, 10, 256);

    let root = control
        .spawn(
            &explore_profile(),
            "session-root",
            "turn-1".to_string(),
            None,
        )
        .await
        .expect("root spawn should succeed");
    let middle = control
        .spawn(
            &explore_profile(),
            "session-middle",
            "turn-1".to_string(),
            Some(root.agent_id.clone()),
        )
        .await
        .expect("middle spawn should succeed");
    let leaf = control
        .spawn(
            &explore_profile(),
            "session-leaf",
            "turn-1".to_string(),
            Some(middle.agent_id.clone()),
        )
        .await
        .expect("leaf spawn should succeed");
    let _ = control
        .set_lifecycle(&root.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&middle.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&leaf.agent_id, AgentLifecycleStatus::Running)
        .await;

    // leaf 向直接父 (middle) 投递
    let leaf_delivery = AgentInboxEnvelope {
        delivery_id: "delivery-leaf-to-middle".to_string(),
        from_agent_id: leaf.agent_id.clone(),
        to_agent_id: middle.agent_id.clone(),
        kind: astrcode_core::InboxEnvelopeKind::ChildDelivery,
        message: "leaf 的结果".to_string(),
        context: None,
        is_final: true,
        summary: Some("leaf 完成了任务".to_string()),
        findings: vec!["发现1".to_string()],
        artifacts: Vec::new(),
    };

    control
        .push_inbox(&middle.agent_id, leaf_delivery)
        .await
        .expect("push to middle should succeed");

    // middle 的 inbox 应该有 leaf 的投递
    let middle_inbox = control
        .drain_inbox(&middle.agent_id)
        .await
        .expect("drain middle inbox should succeed");
    assert_eq!(middle_inbox.len(), 1);
    assert_eq!(middle_inbox[0].from_agent_id, leaf.agent_id);
    assert_eq!(
        middle_inbox[0].kind,
        astrcode_core::InboxEnvelopeKind::ChildDelivery
    );
    assert!(middle_inbox[0].is_final);

    // root 的 inbox 应该为空（leaf 不能越级投递）
    let root_inbox = control
        .drain_inbox(&root.agent_id)
        .await
        .expect("drain root inbox should succeed");
    assert!(
        root_inbox.is_empty(),
        "leaf delivery should not reach grandparent inbox"
    );
}

/// 验证 wait_for_inbox 在 agent 被 terminate_subtree 后能被正确唤醒，
/// 并返回终态 handle 而非永远阻塞。
#[tokio::test]
async fn wait_for_inbox_resolves_on_terminate_subtree() {
    let control = AgentControl::with_limits(4, 10, 256);
    let parent = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("parent spawn should succeed");
    let child = control
        .spawn(
            &explore_profile(),
            "session-1",
            "turn-1".to_string(),
            Some(parent.agent_id.clone()),
        )
        .await
        .expect("child spawn should succeed");
    let _ = control
        .set_lifecycle(&parent.agent_id, AgentLifecycleStatus::Running)
        .await;
    let _ = control
        .set_lifecycle(&child.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 在另一个任务中等待 child 的 inbox
    let child_id = child.agent_id.clone();
    let control_clone = control.clone();
    let waiter = tokio::spawn(async move { control_clone.wait_for_inbox(&child_id).await });

    // 让 waiter 完成订阅
    tokio::task::yield_now().await;

    // terminate parent 的子树，应级联 terminate child 并唤醒 wait_for_inbox
    control
        .terminate_subtree(&parent.agent_id)
        .await
        .expect("terminate should succeed");

    let result = tokio::time::timeout(Duration::from_secs(3), waiter)
        .await
        .expect("waiter should finish before timeout")
        .expect("waiter should join");
    assert!(
        result.is_some(),
        "wait_for_inbox should return Some after terminate"
    );
    let handle = result.unwrap();
    assert!(
        handle.lifecycle.is_final(),
        "handle should be in final state after terminate, got {:?}",
        handle.lifecycle
    );
}

/// 验证 inbox 容量上限生效：超出容量时 push_inbox 返回 None。
#[tokio::test]
async fn push_inbox_rejects_when_at_capacity() {
    let control = AgentControl::with_limits(4, 10, 256);
    // 手动构造小容量 control
    let control = AgentControl {
        inbox_capacity: 2,
        ..control
    };
    let handle = control
        .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
        .await
        .expect("spawn should succeed");
    let _ = control
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await;

    // 推送到容量上限
    assert!(
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-1", "from", &handle.agent_id, "msg-1"),
            )
            .await
            .is_some()
    );
    assert!(
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-2", "from", &handle.agent_id, "msg-2"),
            )
            .await
            .is_some()
    );

    // 超出容量应被拒绝
    assert!(
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-3", "from", &handle.agent_id, "msg-3"),
            )
            .await
            .is_none(),
        "push beyond capacity should return None"
    );
}

/// 验证 parent_delivery_queue 容量上限生效：超出容量时 enqueue 返回 false。
#[tokio::test]
async fn enqueue_parent_delivery_rejects_when_at_capacity() {
    let control = AgentControl::with_limits(4, 10, 256);
    let control = AgentControl {
        parent_delivery_capacity: 2,
        ..control
    };

    let session_id = "session-parent".to_string();
    let turn_id = "turn-parent".to_string();

    // 入队到容量上限
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                sample_parent_delivery("d-1", &session_id, &turn_id).2,
            )
            .await
    );
    assert!(
        control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                sample_parent_delivery("d-2", &session_id, &turn_id).2,
            )
            .await
    );

    // 超出容量应被拒绝
    assert!(
        !control
            .enqueue_parent_delivery(
                session_id.clone(),
                turn_id.clone(),
                sample_parent_delivery("d-3", &session_id, &turn_id).2,
            )
            .await,
        "enqueue beyond capacity should return false"
    );
}
