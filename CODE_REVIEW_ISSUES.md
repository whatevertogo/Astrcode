# Code Review — dev (staged changes)

## Summary
Files reviewed: 47 | New issues: 1 High, 3 Medium, 8 Low | Perspectives: 4/4
Verified false positives filtered: 3 (from agents)

---

## Security
No security issues found.

Session ID validation, auth enforcement, cursor format validation, and child session resolution all properly secured. No SQL/shell injection paths, no hardcoded secrets.

---

## Code Quality
| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Low | `SessionStateEventSink::new` failure silently drops tool metadata | `crates/session-runtime/src/turn/tool_cycle.rs:378-387` | Lost tool metadata events with no diagnostic output |

No critical/high issues. Agent-reported "off-by-one" in `reconcile_tool_chunk` and "race condition" in `complete_turn` verified as **false positives** — the dedup logic correctly handles cross-chunk pending bytes, and `&mut self` in Rust prevents concurrent mutation.

---

## Tests

**New test files present**:
- `crates/protocol/tests/conversation_conformance.rs` (134 lines)
- `crates/protocol/tests/terminal_conformance.rs` (32 lines)
- 4 new JSON fixtures in `crates/protocol/tests/fixtures/`
- Updated `frontend/src/components/Chat/ToolCallBlock.test.tsx`
- Updated `frontend/src/lib/api/conversation.test.ts`

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| High | `tool_result_summary` 4 branches (ok+output, ok-empty, fail+error, fail-empty) | `crates/session-runtime/src/query/conversation.rs:1234-1261` |
| Medium | `reconcile_tool_chunk` edge cases: durable chunks shorter/different boundaries than live | `crates/session-runtime/src/query/conversation.rs:254-278` |
| Medium | `classify_transcript_error` 4 classification branches | `crates/session-runtime/src/query/conversation.rs:1263-1274` |
| Medium | `ChildSessionNotificationKind` -> `ConversationChildHandoffKind` mapping (6->3) | `crates/session-runtime/src/query/conversation.rs:855-869` |
| Low | `execute_concurrent_safe` with `max_concurrency` limit | `crates/session-runtime/src/turn/tool_cycle.rs:278-317` |
| Low | Cancellation during concurrent tool execution | `crates/session-runtime/src/turn/tool_cycle.rs:217-229` |
| Low | `replace_tool_*` no-change detection returning false for unchanged values | `crates/session-runtime/src/query/conversation.rs:994-1058` |
| Low | `complete_turn` for unknown turn_id / turn with only tool blocks | `crates/session-runtime/src/query/conversation.rs:900-928` |
| Low | Frontend `UpdateControlState`, `UpsertChildSummary`, `RehydrateRequired` delta kinds | `frontend/src/lib/api/conversation.test.ts` |
| Low | `ensure_full_markdown_block` replace vs append non-prefix case | `crates/session-runtime/src/query/conversation.rs:586-635` |
| Low | `invoke_single_tool` fallback event buffering on emit failure | `crates/session-runtime/src/turn/tool_cycle.rs:513-527` |

---

## Architecture

Agent-reported "frontend status parsing bug" verified as **false positive** — `ConversationBlockStatusDto` uses `#[serde(rename_all = "snake_case")]` and serializes to plain string `"complete"`, not an object. Frontend `parseToolStatus` correctly matches `case 'complete'`.

No architecture issues found. Cross-layer consistency verified:
- Protocol DTO changes reflected in frontend types and CLI
- ToolStreamBlock removal consistent across all layers
- Server terminal_projection correctly delegates to session-runtime facts
- New `AppSessionPort` methods match session-runtime signatures
- Architecture doc updates match implementation

---

## Must Fix Before Merge
*(None — no critical or high-severity issues that would block merge)*

---

## Pre-Existing Issues (not blocking)
- None identified within scope of this diff

---

## Low-Confidence Observations
- The new `conversation.rs` module (1768 lines) is a large single-file module. As it stabilizes, consider splitting into submodules (tool aggregation, markdown handling, child summaries) for maintainability.
- `child_summary_lookup` inserts the same DTO under multiple keys (child_session_id, open_session_id, session_id). Currently safe but worth noting if session ID semantics diverge in future.
