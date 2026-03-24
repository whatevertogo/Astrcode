# Code Review — protocol-v4-implementation

## Summary
Files reviewed: 29 | New issues: 0 | Perspectives: 4/4

---

## Security
No security issues found.

---

## Code Quality
No new correctness issues found.

Notes:
- During review I found and fixed a real race in `Peer::wait_closed()` where a close notification could be missed if the caller started waiting after the transport had already closed.

---

## Tests
Run results:
- `cargo fmt --all -- --check` passed
- `cargo test -p astrcode-protocol` passed
- `cargo test -p astrcode-sdk` passed
- `cargo test -p astrcode-plugin` passed
- `cargo deny check bans` passed
- `cargo test --workspace` passed

No missing high-signal coverage remains in the new V4 protocol/plugin runtime path. The review-triggered additions cover:
- router profile filtering
- permission checker rejection
- stdio initialize/invoke/result
- started/delta/completed streaming
- cancel propagation
- pending request / pending stream close convergence

---

## Architecture
No new cross-layer inconsistencies found.

The implemented boundary is coherent:
- `protocol` owns wire types only
- `plugin` owns raw transport + peer/router/worker/supervisor runtime
- `sdk` exposes capability-first registration and profile-aware context helpers
- HTTP/SSE contracts remain unchanged

---

## Must Fix Before Merge
None.

---

## Pre-Existing Issues (not blocking)
- Workspace test output still contains existing `dead_code` warnings in `crates/runtime` and `crates/core::test_support`; these were pre-existing and unrelated to the V4 protocol change.

---

## Low-Confidence Observations
None.
