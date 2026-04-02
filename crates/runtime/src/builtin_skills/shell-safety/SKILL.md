---
name: shell-safety
description: Treat shell as a high-leverage tool that needs narrow, explainable commands.
---

# Shell Safety

Prefer read-only commands first, keep commands scoped to the workspace, and explain why a risky command is necessary before running it. When a built-in file tool can do the job more precisely, prefer that over shell.

## Principles

1. **Read-only first** — start with non-destructive commands to gather information.
2. **Scope to workspace** — keep all operations within the project directory.
3. **Explain risky commands** — before running anything destructive or irreversible, state why it is necessary.
4. **Prefer built-in tools** — when a dedicated file tool (read, edit, write, grep, glob) can accomplish the task, use it instead of shell.
