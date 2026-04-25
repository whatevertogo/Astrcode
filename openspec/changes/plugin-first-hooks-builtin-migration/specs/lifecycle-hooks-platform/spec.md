## ADDED Requirements

### Requirement: lifecycle hooks SHALL execute registered handlers
The hooks platform MUST dispatch to registered builtin or external handlers at runtime. Production hook dispatch SHALL NOT be modeled as only precomputed effects.

#### Scenario: matching handler is invoked
- **WHEN** a `tool_call` event is dispatched and a matching active registration exists
- **THEN** the registered handler SHALL be invoked with typed payload
- **AND** dispatch semantics SHALL apply to the handler response

### Requirement: lifecycle hook effects SHALL be validated before owner application
Every hook effect MUST be validated against the event's allowed effect set before an owner applies it.

#### Scenario: invalid effect is returned
- **WHEN** a `turn_end` handler returns `MutateToolArgs`
- **THEN** dispatch SHALL reject the effect
- **AND** failure policy SHALL decide whether to continue, block, or report diagnostic

