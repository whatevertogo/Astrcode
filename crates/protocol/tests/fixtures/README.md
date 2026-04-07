# Protocol Fixture Coverage

This directory records protocol fixture coverage used by conformance tests.

## v4 Baseline Fixtures

- v4/initialize.json: Session initialize payload baseline.
- v4/invoke.json: Tool invocation payload baseline.
- v4/event_delta.json: Streaming delta payload baseline.
- v4/cancel.json: Cancellation payload baseline.
- v4/result_initialize.json: Successful initialize result payload baseline.
- v4/result_error.json: Error result payload baseline.

## Legacy History Coverage Note

Legacy durable subrun lineage behavior is currently validated by runtime/server regression tests
that seed StorageEvent history directly. This fixture directory tracks wire-format payload samples;
legacy lineage degradation semantics are tracked in:

- specs/001-runtime-boundary-refactor/quickstart.md (Scenario C)
- crates/server/src/tests/runtime_routes_tests.rs
- crates/server/src/tests/session_contract_tests.rs
