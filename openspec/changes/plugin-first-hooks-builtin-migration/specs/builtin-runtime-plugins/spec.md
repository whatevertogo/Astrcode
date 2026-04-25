## ADDED Requirements

### Requirement: runtime abilities SHALL be exposed as builtin plugin contributions
Stable in-process abilities visible to the agent or user MUST be described and bound as builtin plugin contributions rather than long-term server hardcoded capability lists.

#### Scenario: descriptor and executor are staged together
- **WHEN** a builtin tool or hook is added
- **THEN** its descriptor and executor binding SHALL be staged as one plugin contribution
- **AND** staging SHALL fail if either side is missing

### Requirement: planning behavior SHALL live in a builtin planning plugin
Plan mode tools, plan mode descriptors, and plan input hooks MUST be contributed by a builtin planning plugin.

#### Scenario: planning tools migrate
- **WHEN** builtin planning plugin is enabled
- **THEN** `enterPlanMode`, `exitPlanMode`, and `upsertSessionPlan` SHALL come from that plugin contribution
- **AND** they SHALL no longer be directly constructed in `build_core_tool_invokers()`

#### Scenario: session plan editing remains explicit
- **WHEN** a planning ability edits the session plan artifact rather than only switching mode
- **THEN** it MAY remain exposed as a tool contribution
- **AND** it SHALL be registered by the builtin planning plugin instead of the core tool list

### Requirement: permission behavior SHALL live in a builtin permission plugin
Default permission enforcement MUST be represented as hook handlers for provider request and tool call checkpoints.

#### Scenario: plan mode denies workspace tool
- **WHEN** current mode policy denies a workspace side-effect tool
- **THEN** the permission plugin SHALL return a tool denial effect during `tool_call`
- **AND** runtime SHALL not execute that tool

### Requirement: composer compact command SHALL be a plugin contribution while compact core stays in context-window
Composer commands and compact customization hooks MUST be plugin contributions, but default compact algorithm MUST remain in `context-window`.

#### Scenario: compact command remains available
- **WHEN** builtin composer plugin is enabled
- **THEN** `compact` command SHALL appear through plugin-host contribution
- **AND** command execution SHALL route to the existing compact owner path
