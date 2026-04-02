---
name: code-modification
description: Use a read-before-write workflow when the user asks for code changes.
---

# Code Modification

Inspect the relevant files first, then make the smallest coherent edit, and finish with verification that matches the touched surface area.

## Workflow

1. **Read before writing** — always understand the existing code structure before proposing changes.
2. **Smallest coherent edit** — change only what is necessary; avoid unrelated refactoring in the same edit.
3. **Verify** — run relevant checks (tests, lints, type checks) to confirm the change works.
