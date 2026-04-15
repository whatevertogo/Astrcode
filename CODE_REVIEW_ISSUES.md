# Code Review -- dev (typed parent delivery for child-to-parent communication)

## Summary
Files reviewed: 12 (core types, application agent mod/routing/terminal/wake, server mapper, adapter-tools send_tool/tests, frontend types) | New issues: 1 (0 critical, 0 high, 1 medium, 0 low) | Perspectives: 4/4

---

## Security

*No security issues found.*

All input flows from LLM tool calls go through `serde_json::from_value` with explicit JSON Schema validation (oneOf with required fields) before reaching the `SendAgentParams` untagged enum. The `send_to_parent` path validates child context, parent existence, and parent lifecycle before any write. No user-controlled unsanitized input reaches SQL/shell/template sinks.

---

## Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Medium | `upstream_collaboration_context` fallback to `child.parent_turn_id` may resolve to a stale turn when the parent's context has a valid `parent_turn_id` but the child's handle carries an outdated one | `crates/application/src/agent/routing.rs:403-419` | Upstream collaboration facts and notifications get written under a wrong parent turn ID; durable event replay and wake reconciliation could misplace the delivery |

### CQ-001: `upstream_collaboration_context` parent_turn_id fallback may be stale

**File**: `crates/application/src/agent/routing.rs:412-415`

```rust
ctx.agent_context()
    .parent_turn_id
    .clone()
    .unwrap_or_else(|| child.parent_turn_id.clone()),
```

The fallback chain prefers the tool context's `parent_turn_id`, but if the child's `AgentEventContext` lacks this field (e.g., it was constructed without it), the code falls back to `child.parent_turn_id` from the `SubRunHandle`. The `SubRunHandle`'s `parent_turn_id` was set at spawn time and never updated. If the parent has since started a new turn (e.g., a wake turn), the child's handle still carries the original parent turn ID.

In practice, this is mitigated by the fact that `ctx.agent_context()` is constructed from the child handle during `submit_prompt_for_agent_with_submission`, which sets `parent_turn_id` from the handle. However, if the child sends an upstream delivery during a resumed turn, the `parent_turn_id` in the context will correctly reflect the *original* parent turn (not a wake turn), which is actually the desired behavior for durable event placement. So the stale-handle concern only manifests if the child's tool context is somehow missing the `parent_turn_id` entirely, which would be a deeper invariant violation.

**Why this is still a real concern**: The fallback silently covers up a missing invariant. If `ctx.agent_context().parent_turn_id` is `None`, the code does not log a warning or fail -- it silently uses the handle's potentially stale value. This makes the invariant hard to detect in production.

**Fix**: Consider adding a `log::warn!` when the fallback path is taken, or requiring `parent_turn_id` to always be present in the tool context for child agents (and returning an error if not).

---

## Tests

**Run results**: Not executed (no build environment available).

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| -- | All new branches have test coverage | -- |

The test suite covers:
- `SendAgentParams` untagged deserialization: downstream shape, upstream shape, missing branch, empty message (all in `crates/adapter-tools/src/agent_tools/tests.rs`)
- Custom `Deserialize` for `SubRunHandoff` and `ChildSessionNotification` with legacy `summary`/`final_reply_excerpt` fields (in `crates/core/src/agent/mod.rs` tests)
- `send_to_parent` rejection for root execution, terminated parent, missing child context (in `crates/application/src/agent/routing.rs` tests)
- `terminal_notification_message` and `terminal_notification_turn_outcome` with typed delivery (in `crates/application/src/agent/mod.rs` tests)
- `project_child_terminal_delivery` preserving explicit envelope (in `crates/application/src/agent/terminal.rs` tests)
- Server mapper `to_subrun_handoff_dto` with typed delivery (in `crates/server/src/http/routes/agents.rs`)
- Frontend `agentEvent.ts`, `applyAgentEvent.ts`, `subRunView.ts` tests

No significant untested branches found in the diff.

---

## Architecture

*No architecture issues found.*

The frontend TypeScript types in `types.ts` correctly mirror the backend `ParentDelivery` discriminated union (kind + payload), including all four variants (progress, completed, failed, close_request). The `SubRunResult.handoff.delivery` field type is properly nullable (`ParentDelivery | undefined`). The `ChildSessionNotificationMessage` in the frontend correctly includes the optional `delivery` field. The protocol DTO types in `astrcode-protocol` align with the core domain types.

---

## Must Fix Before Merge

*No critical or high severity issues. Diff is clear to merge.*

The medium-severity `upstream_collaboration_context` fallback is a defensive observation, not a blocking correctness bug under current usage patterns.

---

## Pre-Existing Issues (not blocking)

- `legacy_notification_delivery` returns `None` when both `summary` and `final_reply_excerpt` are empty/None. This is correct behavior (no useful content to deliver), but downstream code should be aware that `notification.delivery` can be `None` for legacy data with empty fields. The current code handles this correctly in `terminal_notification_message` (fallback message) and `terminal_notification_turn_outcome` (falls through to `notification.kind` match).

---

## Low-Confidence Observations

- **`SendAgentParams` untagged enum ambiguity**: `serde(untagged)` tries variants in declaration order. If `SendToParentParams` (which has `#[serde(flatten)] payload: ParentDeliveryPayload`) could match a `SendToChildParams` JSON, serde would pick `ToChild` first since it's listed first. In practice, `SendToChildParams` requires `agentId` (string) and `message` (string), while `SendToParentParams` requires `kind` (enum) and `payload` (object). These shapes have no field overlap, so the untagged disambiguation is safe. The JSON Schema in `send_tool.rs` also enforces `oneOf` with `additionalProperties: false` on each branch, adding a second layer of protection. **Not a real issue.**

- **`terminal_notification_turn_outcome` Progress delivery returns None**: When `delivery` is `Some(ParentDeliveryPayload::Progress(_))`, the function returns `None` for the turn outcome, then falls through to match on `notification.kind`. This is correct because a Progress delivery is non-terminal and should not override the kind-based outcome inference. **Not a real issue.**
