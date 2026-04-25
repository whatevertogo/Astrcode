## ADDED Requirements

### Requirement: active snapshots SHALL include executable bindings
`plugin-host` active snapshots MUST represent both contribution descriptors and executable bindings for hooks/tools/providers where execution is required.

#### Scenario: descriptor has no binding
- **WHEN** a hook descriptor requires execution but no builtin executor or external backend handler can be resolved
- **THEN** candidate snapshot staging SHALL fail
- **AND** previous active snapshot SHALL remain in effect

#### Scenario: hook snapshot stores handler bindings rather than effects
- **WHEN** plugin-host commits an active snapshot containing hooks
- **THEN** the snapshot SHALL store executable handler bindings
- **AND** it SHALL NOT store precomputed hook effects as the production execution model

### Requirement: builtin and external contributions SHALL share validation rules
Builtin and external plugin contributions MUST be validated through the same uniqueness, schema, event, and capability surface rules.

#### Scenario: duplicate hook id
- **WHEN** a builtin plugin and external plugin declare the same hook id
- **THEN** snapshot validation SHALL reject the candidate
