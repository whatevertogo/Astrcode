## ADDED Requirements

### Requirement: legacy hook DTO cleanup SHALL preserve HookEventKey
The system MUST remove obsolete hook DTOs only after confirming they have no production consumers. `HookEventKey` SHALL remain as the stable event key shared by runtime, host-session, plugin-host, and contract layers.

#### Scenario: obsolete hook DTOs are removed
- **WHEN** `HookEvent`, `ToolHookContext`, `ToolHookResultContext`, `CompactionHookContext`, and `CompactionHookResultContext` have no crate-external production references
- **THEN** they MAY be removed from `core::hook`
- **AND** their `core::lib` re-exports SHALL be removed
- **AND** `HookEventKey` SHALL remain available

#### Scenario: code tries to delete the entire hook module
- **WHEN** cleanup touches `crates/core/src/hook.rs`
- **THEN** implementation SHALL preserve `HookEventKey` or move it to an equivalent shared contract with all call sites migrated
- **AND** cleanup SHALL NOT delete the whole file while `HookEventKey` is still consumed

### Requirement: active governance paths SHALL require replacement before deletion
Server governance, mode compilation, mode catalog, and governance-contract policy paths MUST NOT be deleted until their behavior is replaced by hook/builtin plugin paths and verified by tests.

#### Scenario: server governance surface is still consumed
- **WHEN** `governance_surface`, `governance_service`, `mode/compiler`, `mode/catalog`, `mode_catalog_service`, or `mode/validator` still has production consumers
- **THEN** cleanup SHALL treat it as gated migration work
- **AND** deletion SHALL wait until consumers are migrated to plugin-host snapshot, host-session transition validation, or hook effects

#### Scenario: governance-contract remains referenced
- **WHEN** `governance-contract` types such as `ModeId`, `GovernanceModeSpec`, `SystemPromptBlock`, `PolicyVerdict`, or `ApprovalRequest` are still imported by workspace crates
- **THEN** the crate SHALL NOT be removed
- **AND** implementation SHALL first deduplicate imports and replace only the executable policy path with builtin permission hooks

### Requirement: plan-mode tools SHALL migrate according to behavior, not filename
Plan-related tools MUST be classified by behavior before cleanup. Mode-transition tools may move to hooks or host-session transition entrypoints, while session-plan editing tools may remain tools if they represent user-visible state editing.

#### Scenario: mode transition tool is replaced
- **WHEN** `enterPlanMode` or `exitPlanMode` behavior is fully represented by input hooks or mode transition owner APIs
- **THEN** direct core-tool registration MAY be removed
- **AND** equivalent user-visible mode transition behavior SHALL remain available through the builtin planning plugin

#### Scenario: session plan editing remains a tool
- **WHEN** a tool edits the session plan artifact rather than only switching mode
- **THEN** it MAY remain a tool contribution
- **AND** it SHALL be decoupled from hardcoded mode internals

### Requirement: optional cleanup SHALL be tracked separately from hook execution migration
Workflow, composer option models, and runtime-contract decomposition MUST be treated as optional cleanup unless a task directly depends on them.

#### Scenario: optional cleanup is not required for hook execution
- **WHEN** hook dispatcher, builtin planning plugin, and builtin permission plugin are implemented
- **THEN** success SHALL NOT require deleting `host-session/src/workflow.rs`, `host-session/src/composer.rs`, or `runtime-contract`
- **AND** those cleanups SHALL be scheduled only after separate owner-boundary decisions
