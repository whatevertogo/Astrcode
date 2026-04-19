# Protocol Fixture Coverage

This directory records protocol fixture coverage used by conformance tests.

## v5 Baseline Fixtures

- v5/initialize.json: Session initialize payload baseline.
- v5/invoke.json: Tool invocation payload baseline.
- v5/event_delta.json: Streaming delta payload baseline.
- v5/cancel.json: Cancellation payload baseline.
- v5/result_initialize.json: Successful initialize result payload baseline.
- v5/result_error.json: Error result payload baseline.

## Historical v4 Fixtures

- v4/initialize.json: Session initialize payload baseline.
- v4/invoke.json: Tool invocation payload baseline.
- v4/event_delta.json: Streaming delta payload baseline.
- v4/cancel.json: Cancellation payload baseline.
- v4/result_initialize.json: Successful initialize result payload baseline.
- v4/result_error.json: Error result payload baseline.

## Terminal v1 Fixtures

- terminal/v1/snapshot.json: Authoritative terminal hydration snapshot baseline.
- terminal/v1/delta_append_block.json: Terminal block append delta baseline.
- terminal/v1/delta_patch_block.json: Terminal block patch delta baseline.
- terminal/v1/delta_rehydrate_required.json: Cursor 失效后的 rehydrate-required delta baseline.
- terminal/v1/error_envelope.json: Terminal banner/status error envelope baseline.

## Historical History Coverage Note

Historical durable subrun lineage behavior is currently validated by runtime/server regression tests
that seed StorageEvent history directly. This fixture directory tracks wire-format payload samples;
historical lineage degradation semantics are tracked in:

- specs/001-runtime-boundary-refactor/quickstart.md (Scenario C)
- crates/server/src/tests/runtime_routes_tests.rs
- crates/server/src/tests/session_contract_tests.rs
